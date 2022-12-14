use std::cmp::Ordering;
use std::fmt::{Debug, Formatter};
use std::ops::Range;
use std::time::{Duration, Instant};
use chrono::NaiveTime;
use tokio::runtime::Runtime;
use serde::Deserialize;
use config::PythonBrainConfig;
use working_temp::WorkingTemperatureRange;
use crate::brain::{Brain, BrainFailure, CorrectiveActions};
use crate::brain::python_like::heating_mode::HeatingMode;
use crate::brain::python_like::heating_mode::SharedData;
use crate::io::gpio::GPIOManager;
use crate::io::IOBundle;
use crate::io::robbable::Dispatchable;
use crate::io::temperatures::{Sensor, TemperatureManager};
use crate::io::wiser::hub::{RetrieveDataError, WiserData};
use crate::io::wiser::WiserManager;
use crate::python_like::immersion_heater::ImmersionHeaterModel;
use crate::io::controls::heat_circulation_pump::HeatCirculationPumpControl;
use crate::io::controls::heat_pump::HeatPumpControl;
use crate::io::controls::immersion_heater::ImmersionHeaterControl;
use crate::python_like::overrun_config::{OverrunBap, OverrunConfig};
use crate::time::mytime::get_local_time;
use crate::time::timeslot::ZonedSlot;

pub mod circulate_heat_pump;
pub mod cycling;
pub mod heating_mode;
pub mod immersion_heater;
pub mod config;
mod overrun_config;
mod heatupto;
mod working_temp;

// Functions for getting the max working temperature.

// Was -2, recalibrated by -4 degrees at 35C (true temp), which may be different at different temperatures.
const CALIBRATION_ERROR: f32 = 0.0;

const MAX_ALLOWED_TEMPERATURE: f32 = 55.0 + CALIBRATION_ERROR;

const UNKNOWN_ROOM: &str = "Unknown";

pub trait PythonLikeGPIOManager: GPIOManager + HeatPumpControl + HeatCirculationPumpControl + ImmersionHeaterControl {}

impl<T> PythonLikeGPIOManager for T
    where T: GPIOManager {}

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
    last_successful_contact: Instant,
    shared_data: SharedData,
}

impl PythonBrain {
    pub fn new(config: PythonBrainConfig) -> Self {
        Self {
            shared_data: SharedData::new(FallbackWorkingRange::new(config.get_default_working_range().clone())),

            config,
            heating_mode: HeatingMode::Off,
            last_successful_contact: Instant::now(),
        }
    }
}

impl Default for PythonBrain {
    fn default() -> Self {
        PythonBrain::new(PythonBrainConfig::default())
    }
}

impl Brain for PythonBrain {
    fn run<T, G, W>(&mut self, runtime: &Runtime, io_bundle: &mut IOBundle<T, G, W>) -> Result<(), BrainFailure>
        where T: TemperatureManager, W: WiserManager, G: PythonLikeGPIOManager + Send + 'static {
        let next_mode = self.heating_mode.update(&mut self.shared_data, runtime, &self.config, io_bundle)?;
        if let Some(next_mode) = next_mode {
            println!("Transitioning from {:?} to {:?}", self.heating_mode, next_mode);
            self.heating_mode.transition_to(next_mode, &self.config, runtime, io_bundle)?;
            self.shared_data.notify_entered_state();
        }

        let now = get_local_time();
        let target_sensor = Sensor::TKBT;

        if !matches!(self.heating_mode, HeatingMode::Circulate(_)) {
            let recommended_temp = self.config.get_immersion_heater_model().recommended_temp(now.naive_local().time());
            if let Some(recommend_temp) = recommended_temp {
                println!("Hope for temp {}: {:.2} at this time", target_sensor, recommend_temp);
                let temp = {
                    let temps = io_bundle.temperature_manager().retrieve_temperatures();
                    let temps = runtime.block_on(temps);
                    if temps.is_err() {
                        eprintln!("Error retrieving temperatures: {}", temps.as_ref().unwrap_err());
                    }
                    let temp: Option<f32> = temps.ok().and_then(|m| m.get(&target_sensor).map(|t| *t));
                    temp.clone()
                };
                if let Some(temp) = temp {
                    println!("Current {}: {:.2}", target_sensor, temp);
                    if self.shared_data.immersion_heater_on {
                        if temp > recommend_temp {
                            println!("Turning off immersion heater - reached recommended temp for this time");
                            let gpio = expect_gpio_available(io_bundle.gpio())?;
                            gpio.try_set_immersion_heater(false)?;
                            self.shared_data.immersion_heater_on = false;
                        }
                    } else {
                        if temp < recommend_temp {
                            println!("Turning on immersion heater - in order to reach recommended temp {:.2} (current {:.2})", recommend_temp, temp);
                            let gpio = expect_gpio_available(io_bundle.gpio())?;
                            gpio.try_set_immersion_heater(true)?;
                            self.shared_data.immersion_heater_on = true;
                        }
                    }
                } else if self.shared_data.immersion_heater_on {
                    println!("Turning off immersion heater - no temperatures");
                    let gpio = expect_gpio_available(io_bundle.gpio())?;
                    gpio.try_set_immersion_heater(false)?;
                    self.shared_data.immersion_heater_on = false;
                }
            } else if self.shared_data.immersion_heater_on {
                println!("Turning off immersion heater");
                let gpio = expect_gpio_available(io_bundle.gpio())?;
                gpio.try_set_immersion_heater(false)?;
                self.shared_data.immersion_heater_on = false;
            }
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

fn expect_gpio_available<T: GPIOManager>(dispatchable: &mut Dispatchable<T>) -> Result<&mut T, BrainFailure> {
    if let Dispatchable::Available(gpio) = dispatchable {
        return Ok(&mut *gpio);
    }

    let actions = CorrectiveActions::new().with_gpio_unknown_state();
    return Err(BrainFailure::new("GPIO was not available".to_owned(), actions));
}
