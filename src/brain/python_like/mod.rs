use std::time::{Duration, Instant};
use chrono::{DateTime, Utc};
use tokio::runtime::Runtime;
use config::PythonBrainConfig;
use working_temp::WorkingTemperatureRange;
use crate::brain::{Brain, BrainFailure};
use crate::brain::python_like::heating_mode::HeatingMode;
use crate::brain::python_like::heating_mode::SharedData;
use crate::{brain_fail, get_utc_time, ImmersionHeaterControl};
use crate::io::IOBundle;
use crate::python_like::heating_mode::PossibleTemperatureContainer;
use crate::python_like::immersion_heater::ImmersionHeaterModel;

pub mod cycling;
pub mod heating_mode;
pub mod immersion_heater;
pub mod config;
pub mod control;
pub mod modes;
mod overrun_config;
mod heatupto;
mod working_temp;

// Functions for getting the max working temperature.

// Was -2, recalibrated by -4 degrees at 35C (true temp), which may be different at different temperatures.
const CALIBRATION_ERROR: f32 = 0.0;

const MAX_ALLOWED_TEMPERATURE: f32 = 55.0 + CALIBRATION_ERROR;

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
                println!("Using last working range as fallback: {:?}", range);
                return range;
            }
        }
        println!("No recent previous range to use, using default {:?}", &self.default);
        &self.default
    }

    pub fn update(&mut self, range: WorkingTemperatureRange) {
        self.previous.replace((range, Instant::now()));
    }
}

pub struct PythonBrain {
    config: PythonBrainConfig,
    heating_mode: HeatingMode,
    shared_data: SharedData,
}

impl PythonBrain {
    pub fn new(config: PythonBrainConfig) -> Self {
        Self {
            shared_data: SharedData::new(FallbackWorkingRange::new(config.get_default_working_range().clone())),

            config,
            heating_mode: HeatingMode::Off,
        }
    }
}

impl Default for PythonBrain {
    fn default() -> Self {
        PythonBrain::new(PythonBrainConfig::default())
    }
}

impl Brain for PythonBrain {
    fn run(&mut self, runtime: &Runtime, io_bundle: &mut IOBundle) -> Result<(), BrainFailure> {
        let next_mode = self.heating_mode.update(&mut self.shared_data, runtime, &self.config, io_bundle)?;
        if let Some(next_mode) = next_mode {
            println!("Transitioning from {:?} to {:?}", self.heating_mode, next_mode);
            self.heating_mode.transition_to(next_mode, &self.config, runtime, io_bundle)?;
            self.shared_data.notify_entered_state();
        }

        let temps = runtime.block_on(io_bundle.temperature_manager().retrieve_temperatures());
        if temps.is_err() {
            eprintln!("Error retrieving temperatures: {}", temps.as_ref().unwrap_err());
            if io_bundle.misc_controls().try_get_immersion_heater()? {
                eprintln!("Turning off immersion heater since we didn't get temperatures");
                io_bundle.misc_controls().try_set_immersion_heater(false)?;
            }
            return Ok(());
        }
        let temps = temps.ok().unwrap();
        follow_ih_model(get_utc_time(), &temps, io_bundle.misc_controls().as_ih(), self.config.get_immersion_heater_model())?;

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