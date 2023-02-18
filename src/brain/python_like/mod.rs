use std::time::{Duration, Instant};
use itertools::Itertools;
use tokio::runtime::Runtime;
use config::PythonBrainConfig;
use working_temp::WorkingTemperatureRange;
use crate::brain::{Brain, BrainFailure};
use crate::brain::python_like::boost_active_rooms::{AppliedBoosts, update_boosted_rooms};
use crate::brain::python_like::modes::heating_mode::HeatingMode;
use crate::brain::python_like::modes::heating_mode::SharedData;
use crate::brain::python_like::modes::InfoCache;
use crate::brain::python_like::modes::intention::Intention;
use crate::io::IOBundle;
use crate::brain::python_like::immersion_heater::follow_ih_model;
use crate::time::mytime::TimeProvider;

pub mod cycling;
pub mod immersion_heater;
pub mod config;
pub mod control;
pub mod modes;
mod boost_active_rooms;
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
    applied_boosts: AppliedBoosts,
}

impl PythonBrain {
    pub fn new(config: PythonBrainConfig) -> Self {
        Self {
            shared_data: SharedData::new(FallbackWorkingRange::new(config.get_default_working_range().clone())),
            config,
            heating_mode: None,
            applied_boosts: AppliedBoosts::new(),
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

        let working_temp_range = modes::heating_mode::get_working_temp_fn(&mut self.shared_data.get_fallback_working_range(), io_bundle.wiser(), &self.config, &runtime, time_provider);

        let mut info_cache = InfoCache::create(self.shared_data.last_wiser_state, working_temp_range);

        match &mut self.heating_mode {
            None => {
                println!("No current mode - probably just started up - Running same logic as ending a state.");
                let intention = Intention::finish();
                let new_state = modes::heating_mode::handle_intention(intention, &mut info_cache, io_bundle, &self.config, runtime, &time_provider.get_utc_time())?;
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
            Ok(devices) => {
                println!("Active Devices: {}", devices.iter().map(|dev| dev.get_name()).sorted().format(", "));
                match runtime.block_on(update_boosted_rooms(&mut self.applied_boosts, self.config.get_boost_active_rooms(), devices, io_bundle.wiser())) {
                    Ok(_) => {},
                    Err(error) => {
                        eprintln!("Error boosting active rooms: {}", error);
                    }
                }
            },
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