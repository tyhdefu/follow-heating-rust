use std::cmp::Ordering;
use std::fmt::{Debug, Formatter};
use std::ops::Range;
use std::time::{Duration, Instant};
use chrono::NaiveTime;
use tokio::runtime::Runtime;
use serde::Deserialize;
use config::PythonBrainConfig;
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

// Functions for getting the max working temperature.

// Was -2, recalibrated by -4 degrees at 35C (true temp), which may be different at different temperatures.
const CALIBRATION_ERROR: f32 = 0.0;

const MAX_ALLOWED_TEMPERATURE: f32 = 55.0 + CALIBRATION_ERROR;

const UNKNOWN_ROOM: &str = "Unknown";

fn get_working_temperature(data: &WiserData) -> (WorkingTemperatureRange, f32) {
    let difference = data.get_rooms().iter()
        .filter(|room| room.get_temperature() > -10.0) // Low battery or something.
        .map(|room| (room.get_name().unwrap_or_else(|| UNKNOWN_ROOM), room.get_set_point().min(21.0) - room.get_temperature()))
        .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(Ordering::Equal))
        .unwrap_or_else(|| (UNKNOWN_ROOM, 0.0));

    let range = get_working_temperature_from_max_difference(difference.1);

    if range.get_max() > MAX_ALLOWED_TEMPERATURE {
        eprintln!("Having to cap max temperature from {:.2} to {:.2}", range.max, MAX_ALLOWED_TEMPERATURE);
        let delta = range.get_max() - range.get_min();
        return (WorkingTemperatureRange::from_delta(MAX_ALLOWED_TEMPERATURE, delta), difference.1);
    }
    println!("Working Range {:?} (Room {})", range, difference.0);
    return (range, difference.1);
}

fn get_working_temperature_from_max_difference(difference: f32) -> WorkingTemperatureRange {
    const DIFF_CAP: f32 = 2.5;
    const GRAPH_START_TEMP: f32 = 53.2 + CALIBRATION_ERROR;
    const MULTICAND: f32 = 10.0;
    const LEFT_SHIFT: f32 = 0.6;
    const BASE_RANGE_SIZE: f32 = 4.5;

    let capped_difference = difference.clamp(0.0, DIFF_CAP);
    println!("Difference: {:.2}, Capped: {:.2}", difference, capped_difference);
    let difference = capped_difference;
    let min = GRAPH_START_TEMP - (MULTICAND / (difference + LEFT_SHIFT));
    let max = min + BASE_RANGE_SIZE - difference;
    WorkingTemperatureRange::from_min_max(min, max)
}

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

        if !matches!(self.heating_mode, HeatingMode::Circulate(_)) {
            let recommended_temp = self.config.get_immersion_heater_model().recommended_temp(now.naive_local().time());
            if let Some(recommend_temp) = recommended_temp {
                println!("Hope for temp: {:.2} at this time", recommend_temp);
                let temp = {
                    let temps = io_bundle.temperature_manager().retrieve_temperatures();
                    let temps = runtime.block_on(temps);
                    if temps.is_err() {
                        eprintln!("Error retrieving temperatures: {}", temps.as_ref().unwrap_err());
                    }
                    let temp: Option<f32> = temps.ok().and_then(|m| m.get(&Sensor::TKTP).map(|t| *t));
                    temp.clone()
                };
                if let Some(temp) = temp {
                    println!("Current TKTP: {:.2}", temp);
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

pub fn get_working_temperature_range_from_wiser_data(fallback: &mut FallbackWorkingRange, result: Result<WiserData, RetrieveDataError>) -> (WorkingTemperatureRange, Option<f32>) {
    result.map(|data| {
        let (working_range, max_dist) = get_working_temperature(&data);
        fallback.update(working_range.clone());
        (working_range, Some(max_dist))
    }).unwrap_or_else(|_| (fallback.get_fallback().clone(), None))
}

fn expect_gpio_available<T: GPIOManager>(dispatchable: &mut Dispatchable<T>) -> Result<&mut T, BrainFailure> {
    if let Dispatchable::Available(gpio) = dispatchable {
        return Ok(&mut *gpio);
    }

    let actions = CorrectiveActions::new().with_gpio_unknown_state();
    return Err(BrainFailure::new("GPIO was not available".to_owned(), actions));
}

#[derive(Clone, Deserialize, PartialEq)]
pub struct WorkingTemperatureRange {
    max: f32,
    min: f32,
}

impl WorkingTemperatureRange {
    pub fn from_delta(max: f32, delta: f32) -> Self {
        assert!(delta > 0.0);
        WorkingTemperatureRange {
            max,
            min: max - delta,
        }
    }

    pub fn from_min_max(min: f32, max: f32) -> Self {
        assert!(max > min, "Max should be greater than min.");
        WorkingTemperatureRange {
            max,
            min,
        }
    }
    //271
    pub fn from_config(config: &PythonBrainConfig) -> Self {
        WorkingTemperatureRange::from_delta(config.get_max_heating_hot_water(), config.get_max_heating_hot_water_delta())
    }

    pub fn get_max(&self) -> f32 {
        return self.max;
    }

    pub fn get_min(&self) -> f32 {
        return self.min;
    }

    pub fn modify_max(&mut self, new_max: f32) {
        assert!(self.min < new_max, "New max should be greater than min");
        self.max = new_max;
    }
}

impl Debug for WorkingTemperatureRange {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "WorkingTemperatureRange {{ min: {:.2} max: {:.2} }}", self.min, self.max)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_values() {
        //test_value(500.0, 50.0, 52.0);
        test_value(3.0, 50.0, 52.0);
        test_value(2.5, 50.0, 52.0);
        test_value(2.0, 49.4, 51.9);
        test_value(1.5, 48.4, 51.4);
        test_value(0.5, 44.1, 48.1);
        test_value(0.2, 40.7, 45.0);
        test_value(0.1, 38.9, 43.3);
        test_value(0.0, 36.5, 41.0);
    }

    fn test_value(temp_diff: f32, expect_min: f32, expect_max: f32) {
        const GIVE: f32 = 0.05;
        let expect_min = expect_min + CALIBRATION_ERROR;
        let expect_max = expect_max + CALIBRATION_ERROR;

        let range = get_working_temperature_from_max_difference(temp_diff);
        if !is_within_range(range.get_min(), expect_min, GIVE) {
            panic!("Min value not in range Expected: {} vs Got {} (Give {}) for temp_diff {}", expect_min, range.get_min(), GIVE, temp_diff);
        }
        if !is_within_range(range.get_max(), expect_max, GIVE) {
            panic!("Max value not in range Expected: {} vs Got {} (Give {}) for temp_diff {}", expect_min, range.get_max(), GIVE, temp_diff);
        }
    }

    fn is_within_range(check: f32, expect: f32, give: f32) -> bool {
        return (check - expect).abs() < give;
    }
}
