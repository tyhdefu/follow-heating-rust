use std::time::{Duration, Instant};
use chrono::{DateTime, Utc};
use tokio::runtime::Runtime;
use config::PythonBrainConfig;
use working_temp::WorkingTemperatureRange;
use crate::brain::{Brain, BrainFailure};
use crate::brain::python_like::heating_mode::HeatingMode;
use crate::brain::python_like::heating_mode::SharedData;
use crate::brain::python_like::modes::{InfoCache, Intention};
use crate::ImmersionHeaterControl;
use crate::io::IOBundle;
use crate::python_like::heating_mode::PossibleTemperatureContainer;
use crate::python_like::immersion_heater::ImmersionHeaterModel;
use crate::time::mytime::TimeProvider;

pub mod cycling;
pub mod heating_mode;
pub mod immersion_heater;
pub mod config;
pub mod control;
pub mod modes;
mod overrun_config;
mod working_temp;

// Functions for getting the max working temperature.

const MAX_ALLOWED_TEMPERATURE: f32 = 55.0;

const UNKNOWN_ROOM: &str = "Unknown";

pub struct FallbackWorkingRange {
    previous: Option<(WorkingTemperatureRange, Instant)>,
    default: WorkingTemperatureRange,
}

impl FallbackWorkingRange {
    fn new(default: WorkingTemperatureRange) -> Self {
        FallbackWorkingRange {
            previous: None,
            default,
        }
    }

    pub fn get_fallback(&self) -> &WorkingTemperatureRange {
        const PREVIOUS_RANGE_VALID_FOR: Duration = Duration::from_secs(60 * 30);

        if let Some((range, updated)) = &self.previous {
            if (*updated + PREVIOUS_RANGE_VALID_FOR) > Instant::now() {
                println!("Using last working range as fallback: {}", range);
                return range;
            }
        }
        println!("No recent previous range to use, using default {}", &self.default);
        &self.default
    }

    pub fn update(&mut self, range: WorkingTemperatureRange) {
        self.previous.replace((range, Instant::now()));
    }
}

pub struct PythonBrain {
    config: PythonBrainConfig,
    heating_mode: Option<HeatingMode>,
    shared_data: SharedData,
}

impl PythonBrain {
    pub fn new(config: PythonBrainConfig) -> Self {
        Self {
            shared_data: SharedData::new(FallbackWorkingRange::new(config.get_default_working_range().clone())),

            config,
            heating_mode: None,
        }
    }
}

impl Default for PythonBrain {
    fn default() -> Self {
        PythonBrain::new(PythonBrainConfig::default())
    }
}

impl Brain for PythonBrain {
    fn run(&mut self, runtime: &Runtime, io_bundle: &mut IOBundle, time_provider: &impl TimeProvider) -> Result<(), BrainFailure> {

        // Update our value of wiser's state if possible.
        match runtime.block_on(io_bundle.wiser().get_heating_on()) {
            Ok(wiser_heating_on_new) => {
                self.shared_data.last_successful_contact = Instant::now();
                if self.shared_data.last_wiser_state != wiser_heating_on_new {
                    self.shared_data.last_wiser_state = wiser_heating_on_new;
                    println!("Wiser heating state changed to {}", if wiser_heating_on_new { "On" } else { "Off" });
                }
            }
            Err(_) => {
                // The wiser hub often doesn't respond. If this happens, carry on heating for a maximum of 1 hour.
                eprintln!("Failed to get whether heating was on. Using old value");
                if Instant::now() - self.shared_data.last_successful_contact > Duration::from_secs(60 * 60) {
                    eprintln!("Saying off - last successful contact too long ago: {}s ago", self.shared_data.last_successful_contact.elapsed().as_secs());
                    self.shared_data.last_wiser_state = false;
                }
            }
        }

        let working_temp_range = heating_mode::get_working_temp_fn(&mut self.shared_data.fallback_working_range, io_bundle.wiser(), &self.config, &runtime, time_provider);

        let mut info_cache = InfoCache::create(self.shared_data.last_wiser_state, working_temp_range);

        match &mut self.heating_mode {
            None => {
                println!("No current mode - probably just started up - Running same logic as ending a state.");
                let intention = Intention::finish();
                let new_state = heating_mode::handle_intention(intention, &mut info_cache, io_bundle, &self.config, runtime, &time_provider.get_utc_time())?;
                let mut new_mode = match new_state {
                    None => {
                        eprintln!("Got no next state - should have had something since we didn't keep state. Going to off.");
                        HeatingMode::Off
                    }
                    Some(mode) => mode
                };
                println!("Entering mode: {:?}", new_mode);
                new_mode.enter(&self.config, runtime, io_bundle)?;
                self.heating_mode = Some(new_mode);
                self.shared_data.notify_entered_state();
            }
            Some(cur_mode) => {
                let next_mode = cur_mode.update(&mut self.shared_data, runtime, &self.config, io_bundle, &mut info_cache)?;
                if let Some(next_mode) = next_mode {
                    println!("Transitioning from {:?} to {:?}", cur_mode, next_mode);
                    cur_mode.transition_to(next_mode, &self.config, runtime, io_bundle)?;
                    self.shared_data.notify_entered_state();
                }
            }
        }


        let temps = runtime.block_on(info_cache.get_temps(io_bundle.temperature_manager()));
        if temps.is_err() {
            eprintln!("Error retrieving temperatures: {}", temps.as_ref().unwrap_err());
            if io_bundle.misc_controls().try_get_immersion_heater()? {
                eprintln!("Turning off immersion heater since we didn't get temperatures");
                io_bundle.misc_controls().try_set_immersion_heater(false)?;
            }
            return Ok(());
        }
        let temps = temps.ok().unwrap();
        follow_ih_model(time_provider.get_utc_time(), &temps, io_bundle.misc_controls().as_ih(), self.config.get_immersion_heater_model())?;

        match io_bundle.active_devices().get_active_devices(&time_provider.get_utc_time()) {
            Ok(devices) => println!("Active Devices: {:?}", devices),
            Err(err) => eprintln!("Error getting active devices: {}", err),
        }

        Ok(())
    }

    fn reload_config(&mut self) {
        match config::try_read_python_brain_config() {
            None => eprintln!("Failed to read python brain config, keeping previous config"),
            Some(config) => {
                self.config = config;
                println!("Reloaded config");
            }
        }
    }
}

fn follow_ih_model(time: DateTime<Utc>,
                   temps: &impl PossibleTemperatureContainer,
                   immersion_heater_control: &mut dyn ImmersionHeaterControl,
                   model: &ImmersionHeaterModel,
) -> Result<(), BrainFailure> {
    let currently_on = immersion_heater_control.try_get_immersion_heater()?;
    let recommendation = model.should_be_on(temps, time.naive_local().time());
    if let Some((sensor, recommend_temp)) = recommendation {
        println!("Hope for temp {}: {:.2}, currently {:.2} at this time", sensor, recommend_temp, temps.get_sensor_temp(&sensor).copied().unwrap_or(-10000.0));
        if !currently_on {
            println!("Turning on immersion heater");
            immersion_heater_control.try_set_immersion_heater(true)?;
        }
    } else if currently_on {
        println!("Turning off immersion heater");
        immersion_heater_control.try_set_immersion_heater(false)?;
    }
    Ok(())
}

#[cfg(test)]
mod test {
    use std::collections::HashMap;
    use chrono::TimeZone;
    use crate::{DummyAllOutputs, Sensor};
    use crate::python_like::immersion_heater::ImmersionHeaterModelPart;
    use crate::time::test_utils::{date, time};
    use crate::brain::python_like::control::misc_control::MiscControls;
    use super::*;

    #[test]
    fn check_blank_does_nothing() {
        let mut temps = HashMap::new();
        temps.insert(Sensor::TKTP, 40.0);
        temps.insert(Sensor::TKBT, 20.0);

        let model = ImmersionHeaterModel::new(vec![]);

        let mut dummy = DummyAllOutputs::default();
        let datetime = Utc.from_utc_datetime(&date(2022, 10, 03).and_time(time(02, 30, 00)));
        follow_ih_model(datetime, &temps, dummy.as_ih(), &model).unwrap();

        assert!(!dummy.try_get_immersion_heater().unwrap(), "Immersion heater should have been turned on.");
    }

    #[test]
    fn check_ih_model_follow() {
        let model_part = ImmersionHeaterModelPart::from_time_points(
            (time(00, 30, 00), 30.0),
            (time(04, 30, 00), 38.0),
            Sensor::TKBT,
        );
        let model = ImmersionHeaterModel::new(vec![model_part]);
        let datetime = Utc.from_utc_datetime(&date(2022, 01, 18).and_time(time(02, 30, 00)));
        let mut temps = HashMap::new();
        temps.insert(Sensor::TKTP, 40.0);
        temps.insert(Sensor::TKBT, 32.0);

        let mut dummy = DummyAllOutputs::default();
        follow_ih_model(datetime, &temps, dummy.as_ih(), &model).unwrap();

        assert!(dummy.try_get_immersion_heater().unwrap(), "Immersion heater should have been turned on.");
    }
}