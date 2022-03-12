use std::fs::read;
use std::ops::Add;
use std::thread::sleep;
use std::time::{Duration, Instant};
use chrono::{DateTime, Local, SecondsFormat, Utc};
use tokio::runtime::Runtime;
use tokio::sync::mpsc::{Receiver, Sender};
use tokio::sync::mpsc::error::TryRecvError;
use tokio::task::JoinHandle;
use crate::brain::python_like::{HEAT_CIRCULATION_PUMP, HEAT_PUMP_RELAY, PythonBrainConfig};
use crate::brain::python_like::circulate_heat_pump::{CirculateHeatPumpOnlyTaskHandle, CirculateHeatPumpOnlyTaskMessage};
use crate::io::gpio::{GPIOManager, GPIOState};
use crate::io::robbable::DispatchedRobbable;

pub fn start_task<G>(runtime: &Runtime, gpio: DispatchedRobbable<G>, config: PythonBrainConfig) -> CirculateHeatPumpOnlyTaskHandle
    where G: GPIOManager + Send + 'static {
    let (send, recv) = tokio::sync::mpsc::channel(10);
    let future = cycling_task(config, recv, gpio);
    let handle = runtime.spawn(future);
    CirculateHeatPumpOnlyTaskHandle::new(handle, send)
}
// 1 minute 20 seconds until it will turn on.
async fn cycling_task<G>(config: PythonBrainConfig, mut receiver: Receiver<CirculateHeatPumpOnlyTaskMessage>, gpio_access: DispatchedRobbable<G>)
    where G: GPIOManager {

    // Turn on circulation pump.
    {
        println!("Turning on heat circulation pump");
        let mut lock_result = gpio_access.access().lock().expect("Mutex on gpio is poisoned");
        if lock_result.is_none() {
            println!("Cycling Task - We no longer have the gpio, someone probably robbed it.");
            return;
        }
        let mut gpio = lock_result.as_mut().unwrap();
        gpio.set_pin(HEAT_CIRCULATION_PUMP, &GPIOState::LOW)
            .expect("Should be able to set Heat Pump Relay to High");
    }

    println!("Leaving heat circulation pump on for 60 seconds before continuing");
    if let Some(message) = wait_or_get_message(&mut receiver, Duration::from_secs(60)).await {
        println!("Received message during second part of sleep {:?}", message);
        return;
    }

    println!("Starting the main cycling task loop.");
    loop {
        // Turn on gpio.
        println!("Turning on heat pump.");
        set_heat_pump_state(&gpio_access, true);

        println!("Waiting {:?}", config.hp_pump_on_time);
        if let Some(message) = wait_or_get_message(&mut receiver, config.hp_pump_on_time).await {
            println!("Received message during while on {:?}", message);
            if message.leave_on() {
                // Do nothing.
            }
            else {
                set_heat_pump_state(&gpio_access, false);
            }
            return;
        }

        println!("Turning off heat pump");
        set_heat_pump_state(&gpio_access, false);

        println!("Waiting {:?}", config.hp_pump_off_time);
        if let Some(message) = wait_or_get_message(&mut receiver, config.hp_pump_off_time).await {
            println!("Received message during while off {:?}", message);
            return;
        }
    }

    fn set_heat_pump_state<G>(robbable: &DispatchedRobbable<G>, on: bool)
        where G: GPIOManager {
        let mut lock_result = robbable.access().lock().expect("Mutex on gpio is poisoned");
        if lock_result.is_none() {
            println!("Cycling Task - We no longer have the gpio, someone probably robbed it.");
            return;
        }
        let mut gpio = lock_result.as_mut().unwrap();
        let state = if on { GPIOState::LOW } else {GPIOState::HIGH };
        gpio.set_pin(HEAT_PUMP_RELAY, &state)
            .expect("Should be able to set Heat Pump Relay to High");
    }

    async fn wait_or_get_message(receiver: &mut Receiver<CirculateHeatPumpOnlyTaskMessage>, wait: Duration) -> Option<CirculateHeatPumpOnlyTaskMessage> {
        let result = tokio::time::timeout(wait, receiver.recv()).await;
        match result {
            Ok(None) => panic!("Other side disconnected"),
            Ok(Some(message)) => Some(message),
            Err(_timeout) => None
        }
    }
}