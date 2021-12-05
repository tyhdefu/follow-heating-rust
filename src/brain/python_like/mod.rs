use std::cmp::{max, Ordering};
use std::fmt::{Debug, Formatter};
use std::time::{Duration, Instant};
use chrono::{DateTime, NaiveTime};
use futures::{FutureExt};
use tokio::runtime::Runtime;
use crate::brain::{Brain, BrainFailure, CorrectiveActions};
use crate::brain::python_like::cycling::CyclingTaskHandle;
use crate::brain::python_like::HeatingMode::Cycling;
use crate::io::gpio::{GPIOError, GPIOManager, GPIOState};
use crate::io::IOBundle;
use crate::io::robbable::Dispatchable;
use crate::io::temperatures::{Sensor, TemperatureManager};
use crate::io::wiser::hub::{RetrieveDataError, WiserData};
use crate::io::wiser::WiserManager;

pub mod cycling;

pub const HEAT_PUMP_RELAY: usize = 26;
pub const HEAT_CIRCULATION_PUMP: usize = 5;

// Functions for getting the max working temperature.

const MAX_ALLOWED_TEMPERATURE: f32 = 53.0; // 55 actual

fn get_working_temperature(data: &WiserData) -> WorkingTemperatureRange {
    let difference = data.get_rooms().iter()
        .filter(|room| room.get_temperature() > -10.0) // Low battery or something.
        .map(|room| get_min_float(21.0, room.get_set_point()) - room.get_temperature())
        .max_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal))
        .unwrap_or(0.0);

    let range =  get_working_temperature_from_max_difference(difference);

    if range.get_max() > MAX_ALLOWED_TEMPERATURE {
        eprintln!("Having to cap max temperature from {:.2} to {:.2}", range.max, MAX_ALLOWED_TEMPERATURE);
        let delta = range.get_max() - range.get_min();
        return WorkingTemperatureRange::from_delta(MAX_ALLOWED_TEMPERATURE, delta);
    }
    println!("Working Range {:?}", range);
    return range;
}

const CALIBRATION_ERROR: f32 = -2.0;

fn get_working_temperature_from_max_difference(difference: f32) -> WorkingTemperatureRange {
    const DIFF_CAP: f32 = 2.5;
    const GRAPH_START_TEMP: f32 = 53.2 + CALIBRATION_ERROR;
    const MULTICAND: f32 = 10.0;
    const LEFT_SHIFT: f32 = 0.6;
    const BASE_RANGE_SIZE: f32 = 4.5;

    let capped_difference = get_max_float(0.0, get_min_float(DIFF_CAP, difference));
    println!("Difference: {:.2}, Capped: {:.2}", difference, capped_difference);
    let difference = capped_difference;
    let min = GRAPH_START_TEMP - (MULTICAND/(difference + LEFT_SHIFT));
    let max =  min+BASE_RANGE_SIZE-difference;
    WorkingTemperatureRange::from_min_max(min, max)
}

fn get_min_float(a: f32, b: f32) -> f32 {
    if a < b {
        return a;
    }
    b
}

fn get_max_float(a: f32, b: f32) -> f32 {
    if a > b {
        return a;
    }
    b
}

#[derive(Clone)]
pub struct PythonBrainConfig {
    hp_pump_on_time: Duration,
    hp_pump_off_time: Duration,
    hp_fully_reneable_min_time: Duration,

    max_heating_hot_water: f32,
    max_heating_hot_water_delta: f32,
    temp_before_circulate: f32,

    try_not_to_turn_on_heat_pump_after: NaiveTime,
    try_not_to_turnon_heat_pump_end_threshold: Duration,
    try_not_to_turn_on_heat_pump_extra_delta: f32,

    initial_heat_pump_cycling_sleep: Duration,
    default_working_range: WorkingTemperatureRange,
}

impl Default for PythonBrainConfig {
    fn default() -> Self {
        PythonBrainConfig {
            hp_pump_on_time: Duration::from_secs(70),
            hp_pump_off_time: Duration::from_secs(100),
            hp_fully_reneable_min_time: Duration::from_secs(15 * 60),
            max_heating_hot_water: 42.0,
            max_heating_hot_water_delta: 5.0,
            temp_before_circulate: 33.0,
            try_not_to_turn_on_heat_pump_after: NaiveTime::from_hms(19, 30, 0),
            try_not_to_turnon_heat_pump_end_threshold: Duration::from_secs(20 * 60),
            try_not_to_turn_on_heat_pump_extra_delta: 5.0,
            initial_heat_pump_cycling_sleep: Duration::from_secs(5 * 60),
            default_working_range: WorkingTemperatureRange::from_min_max(42.0, 45.0),
        }
    }
}

struct FallbackWorkingRange {
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
        const PREVIOUS_RANGE_VALID_FOR: Duration = Duration::from_secs(60*30);

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

enum HeatingMode {
    Off,
    On,
    Cycling(CyclingTaskHandle),
}

pub struct PythonBrain {
    config: PythonBrainConfig,
    heating_mode: HeatingMode,
    last_successful_contact: Instant,
    fallback_working_range: FallbackWorkingRange,
}

impl PythonBrain {

    pub fn new() -> Self {
        let config = PythonBrainConfig::default();
        PythonBrain {
            fallback_working_range: FallbackWorkingRange::new(config.default_working_range.clone()),
            config,
            heating_mode: HeatingMode::Off,
            last_successful_contact: Instant::now(),
        }
    }
}

impl Brain for PythonBrain {
    fn run<T, G, W>(&mut self, runtime: &Runtime, io_bundle: &mut IOBundle<T, G, W>) -> Result<(), BrainFailure>
        where T: TemperatureManager, W: WiserManager, G: GPIOManager + Send + 'static {
        let heating_on_result = runtime.block_on(io_bundle.wiser().get_heating_on());
        // The wiser hub often doesn't respond. If this happens, carry on heating for a maximum of 1 hour.
        if heating_on_result.is_ok() {
            self.last_successful_contact = Instant::now();
        }
        let heating_on = heating_on_result.unwrap_or_else(|e| {
            if Instant::now() - self.last_successful_contact > Duration::from_secs(60*60) {
                return false;
            }
            match self.heating_mode {
                HeatingMode::Off => false,
                HeatingMode::On => true,
                Cycling(_) => true,
            }
        });
        if heating_on {
            if let HeatingMode::Off = &self.heating_mode {
                // Activate heating.
                println!("Heating turned on, turning on gpios.");
                let gpio = expect_gpio_available(io_bundle.gpio())?;
                gpio.set_pin(HEAT_PUMP_RELAY, &GPIOState::LOW).expect("Failed to set pin.");
                gpio.set_pin(HEAT_CIRCULATION_PUMP, &GPIOState::LOW).expect("Failed to set pin");
                self.heating_mode = HeatingMode::On
            }
        }
        else {
            if let HeatingMode::On = &self.heating_mode  {
                // Turn off.
                println!("Heating turned off, turning off gpios.");
                let gpio = expect_gpio_available(io_bundle.gpio())?;
                gpio.set_pin(HEAT_PUMP_RELAY, &GPIOState::HIGH)
                    .map_err(|err| BrainFailure::new(format!("Failed to turn off Heat Pump GPIO after cycling {:?}", err), CorrectiveActions::unknown_gpio()))?;
                gpio.set_pin(HEAT_CIRCULATION_PUMP, &GPIOState::HIGH)
                    .map_err(|err| BrainFailure::new(format!("Failed to turn off Heat Circulation Pump GPIO after cycling {:?}", err), CorrectiveActions::unknown_gpio()))?;

                self.heating_mode = HeatingMode::Off;
            }
            else if let Cycling(task) = &mut self.heating_mode {
                if task.get_sent_terminate_request().is_none() {
                    println!("Heating turned off, terminating cycling, and leaving off");
                    task.terminate_soon(false);
                }
            }
        }

        let temps = io_bundle.temperature_manager().retrieve_temperatures();
        let temps = runtime.block_on(temps);
        if temps.is_err() {
            println!("Error retrieving temperatures: {}", temps.unwrap_err());
            return Ok(());
        }
        let temps = temps.unwrap();

        let get_wiser_data = |wiser: &W| {
            let wiser_data = runtime.block_on(wiser.get_wiser_hub().get_data());
            if wiser_data.is_err() {
                println!("Failed to retrieve wiser data {:?}", wiser_data.as_ref().unwrap_err());
            }
            wiser_data
        };

        if let HeatingMode::On = &self.heating_mode {
            let wiser_data = get_wiser_data(&io_bundle.wiser());
            if let Some(tkbt) = temps.get(&Sensor::TKBT) {
                println!("TKBT: {:.2}", tkbt);
                let max_heating_hot_water = get_working_temperature_range_from_wiser_data(&mut self.fallback_working_range, wiser_data);
                if *tkbt > max_heating_hot_water.get_max() {
                    println!("Reached above {:.2} at TKBT, turning off and will begin cycling.", max_heating_hot_water.max);
                    if let Dispatchable::Available(gpio) = io_bundle.gpio() {
                        gpio.set_pin(HEAT_PUMP_RELAY, &GPIOState::HIGH)
                            .map_err(|err| BrainFailure::new(format!("Failed to turn off Heat Pump GPIO when we reached temperature {:?}", err), CorrectiveActions::unknown_gpio()))?;
                        gpio.set_pin(HEAT_CIRCULATION_PUMP, &GPIOState::HIGH)
                            .map_err(|err| BrainFailure::new(format!("Failed to turn off Heat Circulation Pump GPIO when we reached temperature {:?}", err), CorrectiveActions::unknown_gpio()))?;
                    }
                    else {
                        return Err(BrainFailure::new("GPIO wasn't available when we wanted to dispatch it.".to_owned(), CorrectiveActions::new().with_gpio_unknown_state()));
                    }
                    let dispatched = io_bundle.dispatch_gpio().unwrap(); // We just checked and it was available
                    let handle = CyclingTaskHandle::start_task(runtime, dispatched, self.config.clone(), self.config.initial_heat_pump_cycling_sleep);
                    self.heating_mode = Cycling(handle);
                }
            } else {
                println!("No TKBT returned when we tried to retrieve temperatures. Returned sensors: {:?}", temps);
            }
        }

        if let Cycling(task) = &mut self.heating_mode {
            let wiser_data = get_wiser_data(io_bundle.wiser());
            if let Some(value) = task.join_handle().now_or_never() {
                if value.is_err() {
                    panic!("Join Handle returned an error! {}", value.unwrap_err());
                }
                println!("We have been returned the gpio!");
                let gpio = io_bundle.gpio().rob_or_get_now()
                    .map_err(|err| BrainFailure::new(format!("Cycling task panicked, and left the gpio manager in a potentially unusable state {:?}", err), CorrectiveActions::unknown_gpio()))?;
                if heating_on {
                    println!("After Cycling - The heating is on, making sure it is on.");
                    self.heating_mode = HeatingMode::On;
                    let heat_pump_state = gpio.get_pin(HEAT_PUMP_RELAY)
                        .map_err(|err| BrainFailure::new(format!("Failed to get state of Heat Pump {:?}", err), CorrectiveActions::unknown_gpio()))?;
                    if let GPIOState::HIGH = heat_pump_state {
                        gpio.set_pin(HEAT_PUMP_RELAY, &GPIOState::LOW)
                            .map_err(|err| BrainFailure::new(format!("Failed to turn on Heat Pump {:?}", err), CorrectiveActions::unknown_gpio()))?;
                    }

                    let heat_circulation_pump_state = gpio.get_pin(HEAT_CIRCULATION_PUMP)
                        .map_err(|err| BrainFailure::new(format!("Failed to get state of Heat Pump {:?}", err), CorrectiveActions::unknown_gpio()))?;
                    if let GPIOState::HIGH = heat_circulation_pump_state {
                        gpio.set_pin(HEAT_CIRCULATION_PUMP, &GPIOState::LOW)
                            .map_err(|err| BrainFailure::new(format!("Failed to turn on Heat Circulation Pump {:?}", err), CorrectiveActions::unknown_gpio()))?;
                    }
                }
                else {
                    println!("After Cycling - The heating is off, turning it off.");
                    self.heating_mode = HeatingMode::Off;
                    gpio.set_pin(HEAT_PUMP_RELAY, &GPIOState::HIGH)
                        .map_err(|err| BrainFailure::new(format!("Failed to turn off Heat Pump {:?}", err), CorrectiveActions::unknown_gpio()))?;
                    gpio.set_pin(HEAT_CIRCULATION_PUMP, &GPIOState::HIGH)
                        .map_err(|err| BrainFailure::new(format!("Failed to turn off Heat Pump {:?}", err), CorrectiveActions::unknown_gpio()))?;
                }
            }
            else if let Some(when) = task.get_sent_terminate_request() {
                let allow_time = std::cmp::max(self.config.hp_pump_on_time, self.config.hp_pump_off_time) + Duration::from_secs(20);
                if Instant::now() - *when > allow_time {
                    // Some kind of issue here.
                    task.join_handle().abort();
                    return Err(BrainFailure::new("Didn't get back GPIO from cycling thread".to_owned(), CorrectiveActions::unknown_gpio()));
                }
            }
            else {
                if let Some(tkbt) = temps.get(&Sensor::TKBT) {
                    println!("TKBT: {:.2}", tkbt);
                    let range = get_working_temperature_range_from_wiser_data(&mut self.fallback_working_range, wiser_data);
                    if *tkbt < range.get_min() {
                        println!("Reached below {:.2} at TKBT, stopping cycling and turning on properly.", range.get_min());
                        task.terminate_soon(true);
                    }
                } else {
                    println!("No TKBT returned when we tried to retrieve temperatures. Returned sensors: {:?}", temps);
                }
            }
        }
        Ok(())
    }
}

fn get_working_temperature_range_from_wiser_data(fallback: &mut FallbackWorkingRange, result: Result<WiserData, RetrieveDataError>) -> WorkingTemperatureRange {
    result.map(|data| {
        let working_range = get_working_temperature(&data);
        fallback.update(working_range.clone());
        working_range
    }).unwrap_or_else(|_| fallback.get_fallback().clone())
}

fn expect_gpio_available<T: GPIOManager>(dispatchable: &mut Dispatchable<T>) -> Result<&mut T, BrainFailure> {
    if let Dispatchable::Available(gpio) = dispatchable {
        return Ok(&mut *gpio);
    }

    let actions = CorrectiveActions::new().with_gpio_unknown_state();
    return Err(BrainFailure::new("GPIO was not available".to_owned(), actions));
}

#[derive(Clone)]
struct WorkingTemperatureRange {
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
            min
        }
    }
//271
    pub fn from_config(config: &PythonBrainConfig) -> Self {
        WorkingTemperatureRange::from_delta(config.max_heating_hot_water, config.max_heating_hot_water_delta)
    }

    pub fn get_max(&self) -> f32 {
        return self.max;
    }

    pub fn get_min(&self) -> f32 {
        return self.min;
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