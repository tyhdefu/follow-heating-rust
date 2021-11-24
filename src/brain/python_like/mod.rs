use std::cmp::{max, Ordering};
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
use crate::io::wiser::hub::WiserData;
use crate::io::wiser::WiserManager;

pub mod cycling;

pub const HEAT_PUMP_RELAY: usize = 26;
pub const HEAT_CIRCULATION_PUMP: usize = 5;

const HEAT_PUMP_DB_ID: usize = 13;
const HEAT_CIRCULATION_PUMP_DB_ID: usize = 14;

const HEATING_DB_ID: usize = 17;

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
}

impl Default for PythonBrainConfig {
    fn default() -> Self {
        PythonBrainConfig {
            hp_pump_on_time: Duration::from_secs(1 * 60),
            hp_pump_off_time: Duration::from_secs(2 * 60),
            hp_fully_reneable_min_time: Duration::from_secs(15 * 60),
            max_heating_hot_water: 46.0,
            max_heating_hot_water_delta: 5.0,
            temp_before_circulate: 33.0,
            try_not_to_turn_on_heat_pump_after: NaiveTime::from_hms(19, 30, 0),
            try_not_to_turnon_heat_pump_end_threshold: Duration::from_secs(20 * 60),
            try_not_to_turn_on_heat_pump_extra_delta: 5.0,
        }
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
}

impl PythonBrain {

    pub fn new() -> Self {
        PythonBrain {
            config: PythonBrainConfig::default(),
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
                let mut gpio = expect_gpio_available(io_bundle.gpio())?;
                gpio.set_pin(HEAT_PUMP_RELAY, &GPIOState::LOW).expect("Failed to set pin.");
                gpio.set_pin(HEAT_CIRCULATION_PUMP, &GPIOState::LOW).expect("Failed to set pin");
                self.heating_mode = HeatingMode::On
            }
        }
        else {
            if let HeatingMode::On = &self.heating_mode  {
                // Turn off.
                println!("Heating turned off, turning off gpios.");
                let mut gpio = expect_gpio_available(io_bundle.gpio())?;
                gpio.set_pin(HEAT_PUMP_RELAY, &GPIOState::HIGH);
                gpio.set_pin(HEAT_CIRCULATION_PUMP, &GPIOState::HIGH);

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

        if let HeatingMode::On = &self.heating_mode {
            let wiser_data = runtime.block_on(io_bundle.wiser().get_wiser_hub().get_data());
            if wiser_data.is_err() {
                println!("Failed to retrieve wiser data {:?}", wiser_data.as_ref().unwrap_err());
            }
            if let Some(tkbt) = temps.get(&Sensor::TKBT) {
                let max_heating_hot_water = match &wiser_data {
                    Ok(data) => get_max_temperature(data, &self.config),
                    _ => WorkingTemperatureRange::from_config(&self.config),
                };
                if *tkbt > max_heating_hot_water.max {
                    println!("Reached {} at TKBT, turning off and will begin cycling.", max_heating_hot_water.max);
                    if let Dispatchable::Available(gpio) = io_bundle.gpio() {
                        gpio.set_pin(HEAT_PUMP_RELAY, &GPIOState::HIGH)
                            .map_err(|err| BrainFailure::new(format!("Failed to turn off Heat Pump GPIO when we reached temperature {:?}", err), CorrectiveActions::unknown_gpio()))?;
                    }
                    else {
                        return Err(BrainFailure::new("GPIO wasn't available when we wanted to dispatch it.".to_owned(), CorrectiveActions::new().with_gpio_unknown_state()));
                    }
                    let dispatched = io_bundle.dispatch_gpio().unwrap(); // We just checked and it was available
                    let handle = CyclingTaskHandle::start_task(runtime, dispatched, self.config.clone(), false);
                    self.heating_mode = Cycling(handle);
                }
            } else {
                println!("No TKBT returned when we tried to retrieve temperatures. Returned sensors: {:?}", temps);
            }
        }

        if let Cycling(task) = &mut self.heating_mode {
            let wiser_data = runtime.block_on(io_bundle.wiser().get_wiser_hub().get_data());
            if wiser_data.is_err() {
                println!("Failed to retrieve wiser data {:?}", wiser_data.as_ref().unwrap_err());
            }
            if let Some(value) = task.join_handle().now_or_never() {
                if value.is_err() {
                    panic!("Join Handle returned an error! {}", value.unwrap_err());
                }
                println!("We have been returned the gpio!");
                let mut gpio = io_bundle.gpio().rob_or_get_now()
                    .map_err(|err| BrainFailure::new(format!("Cycling task panicked, and left the gpio manager in a potentially unusable state {:?}", err), CorrectiveActions::unknown_gpio()))?;
                match gpio.get_pin(HEAT_PUMP_RELAY).expect("Change this later..") {
                    GPIOState::HIGH => self.heating_mode = HeatingMode::Off,
                    GPIOState::LOW => self.heating_mode = HeatingMode::On,
                }
                if let HeatingMode::Off = self.heating_mode {
                    gpio.set_pin(HEAT_PUMP_RELAY, &GPIOState::HIGH)
                        .map_err(|err| BrainFailure::new(format!("Failed to turn off Heat Pump GPIO {:?}", err), CorrectiveActions::unknown_gpio()))?;
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
                    let range = match &wiser_data {
                        Ok(data) => get_max_temperature(data, &self.config),
                        _ => WorkingTemperatureRange::from_config(&self.config),
                    };
                    if *tkbt < range.get_min() {
                        println!("Reached {} at TKBT, stopping cycling and turning on properly.", range.get_min());
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

fn expect_gpio_available<T: GPIOManager>(dispatchable: &mut Dispatchable<T>) -> Result<&mut T, BrainFailure> {
    if let Dispatchable::Available(gpio) = dispatchable {
        return Ok(&mut *gpio);
    }

    let actions = CorrectiveActions::new().with_gpio_unknown_state();
    return Err(BrainFailure::new("GPIO was not available".to_owned(), actions));
}

const MAX_ALLOWED_TEMPERATURE: f32 = 49.0;

#[derive(Debug)]
struct WorkingTemperatureRange {
    max: f32,
    delta: f32,
}

impl WorkingTemperatureRange {
    pub fn new(max: f32, delta: f32) -> Self {
        assert!(delta > 0.0);
        WorkingTemperatureRange {
            max,
            delta,
        }
    }

    pub fn from_config(config: &PythonBrainConfig) -> Self {
        WorkingTemperatureRange::new(config.max_heating_hot_water, config.max_heating_hot_water_delta)
    }

    pub fn get_min(&self) -> f32 {
        return self.max - self.delta;
    }
}

fn get_max_temperature(data: &WiserData, config: &PythonBrainConfig) -> WorkingTemperatureRange {
    let temp = data.get_rooms().iter()
        .map(|room| get_min_float(20.0, room.get_set_point()) - room.get_temperature())
        .filter(|distance| *distance < 100.0) // Low battery or something.
        .max_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal))
        .map(|distance| {
            if distance <= 0.0 {
                return WorkingTemperatureRange::from_config(config);
            }
            let mut max = config.max_heating_hot_water + distance;
            let mut delta = config.max_heating_hot_water_delta;
            if max > MAX_ALLOWED_TEMPERATURE {
                max = MAX_ALLOWED_TEMPERATURE;
                delta = config.max_heating_hot_water_delta - 1.0;
            }
            WorkingTemperatureRange::new(max, config.max_heating_hot_water_delta)
        })
        .unwrap_or(WorkingTemperatureRange::from_config(config));
    if temp.max > MAX_ALLOWED_TEMPERATURE {
        println!("Capping max temperature from {} to {}", temp.max, MAX_ALLOWED_TEMPERATURE);
        return WorkingTemperatureRange::new(MAX_ALLOWED_TEMPERATURE, temp.delta);
    }
    println!("Working Range {:?}", temp);
    return temp;
}

fn get_min_float(a: f32, b: f32) -> f32 {
    if a < b {
        return a;
    }
    b
}