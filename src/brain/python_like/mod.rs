use std::borrow::BorrowMut;
use std::collections::HashMap;
use std::ops::Deref;
use std::sync::{Arc, mpsc, Mutex, MutexGuard, TryLockError, TryLockResult};
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::time::{Duration, Instant};
use chrono::{DateTime, NaiveTime};
use futures::{FutureExt, TryFutureExt};
use tokio::runtime::Runtime;
use tokio::task::JoinHandle;
use crate::brain::{Brain, BrainFailure, CorrectiveActions};
use crate::brain::python_like::cycling::CyclingTaskHandle;
use crate::brain::python_like::HeatingMode::Cycling;
use crate::io::gpio::{GPIOError, GPIOManager, GPIOState};
use crate::io::IOBundle;
use crate::io::temperatures::{Sensor, TemperatureManager};
use crate::io::wiser::WiserManager;

pub mod cycling;

const HEAT_PUMP_RELAY: usize = 26;
const HEAT_CIRCULATION_PUMP: usize = 5;

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

enum HeatingMode<G>
    where G: GPIOManager + Send {
    Off,
    On,
    Cycling(CyclingTaskHandle<G>),
}

pub struct PythonBrain<G>
    where G: GPIOManager + Send {
    config: PythonBrainConfig,
    heating_mode: HeatingMode<G>,
}

impl<G> PythonBrain<G>
    where G: GPIOManager + Send {

    pub fn new() -> Self {
        PythonBrain {
            config: PythonBrainConfig::default(),
            heating_mode: HeatingMode::Off,
        }
    }
}

impl<G> Brain<G> for PythonBrain<G>
    where G: GPIOManager + Send + 'static {
    fn run<T, W>(&mut self, runtime: &Runtime, io_bundle: &mut IOBundle<T, G, W>) -> Result<(), BrainFailure>
        where T: TemperatureManager, W: WiserManager {
        let heating_on = io_bundle.wiser().get_heating_on();
        if heating_on {
            if let HeatingMode::Off = &self.heating_mode {
                // Activate heating.
                let mut gpio = io_bundle.gpio().as_mut().expect("GPIO should be available");
                gpio.set_pin(HEAT_PUMP_RELAY, &GPIOState::LOW).expect("Failed to set pin.");
                gpio.set_pin(HEAT_CIRCULATION_PUMP, &GPIOState::LOW).expect("Failed to set pin");
                self.heating_mode = HeatingMode::On
            }
        }
        else {
            if let HeatingMode::On = &self.heating_mode  {
                // Turn off.
                let mut gpio = io_bundle.gpio().as_mut().expect("GPIO should be available");
                gpio.set_pin(HEAT_PUMP_RELAY, &GPIOState::HIGH);
                gpio.set_pin(HEAT_CIRCULATION_PUMP, &GPIOState::HIGH);
                self.heating_mode = HeatingMode::Off;
            }
            else if let Cycling(task) = &mut self.heating_mode {
                if task.get_sent_terminate_request().is_none() {
                    println!("Heating turned off, terminating cycling, and leaving off");
                    task.terminate(false);
                }
            }
        }

        let temps = io_bundle.temperature_manager().retrieve_temperatures();
        let temps = futures::executor::block_on(temps);
        if temps.is_err() {
            println!("Error retrieving temperatures: {}", temps.unwrap_err());
            return Ok(());
        }
        let temps = temps.unwrap();

        if let HeatingMode::On = &self.heating_mode {
            if let Some(tkbt) = temps.get(&Sensor::TKBT) {
                if *tkbt > self.config.max_heating_hot_water {
                    println!("Reached {} at TKBT, turning off and will begin cycling.", self.config.max_heating_hot_water);
                    let mut gpio = io_bundle.gpio().take().unwrap();
                    gpio.set_pin(HEAT_PUMP_RELAY, &GPIOState::HIGH);
                    let handle = CyclingTaskHandle::start_task(runtime, gpio, self.config.clone(), false);
                    self.heating_mode = Cycling(handle);
                }
            } else {
                println!("No TKBT returned when we tried to retrieve temperatures. Returned sensors: {:?}", temps);
            }
        }

        if let Cycling(task) = &mut self.heating_mode {
            if let Some(Ok(gpio)) = task.join_handle().now_or_never() {
                println!("We have been returned the gpio!");
                // TODO: Don't expect.
                match gpio.get_pin(HEAT_PUMP_RELAY).expect("Change this later..") {
                    GPIOState::HIGH => self.heating_mode = HeatingMode::Off,
                    GPIOState::LOW => self.heating_mode = HeatingMode::On,
                }
                io_bundle.gpio().replace(gpio);
                if let HeatingMode::Off = self.heating_mode {
                    io_bundle.gpio().as_mut().unwrap().set_pin(HEAT_CIRCULATION_PUMP, &GPIOState::HIGH);
                }
            }
            else if let Some(when) = task.get_sent_terminate_request() {
                let allow_time = std::cmp::max(self.config.hp_pump_on_time, self.config.hp_pump_off_time) + Duration::from_secs(20);
                if Instant::now() - *when > allow_time {
                    // Some kind of issue here.
                    task.join_handle().abort();
                    let corrective_actions = CorrectiveActions::new()
                        .with_gpio_unknown_state();
                    return Err(BrainFailure::new("Didn't get back GPIO from cycling thread".to_owned(), corrective_actions));
                }
            }
            else {
                if let Some(tkbt) = temps.get(&Sensor::TKBT) {
                    if *tkbt < (self.config.max_heating_hot_water - self.config.max_heating_hot_water_delta) {
                        println!("Reached {} at TKBT, stopping cycling and turning on properly.", self.config.max_heating_hot_water);
                        task.terminate(true);
                    }
                } else {
                    println!("No TKBT returned when we tried to retrieve temperatures. Returned sensors: {:?}", temps);
                }
            }
        }
        Ok(())
    }
}