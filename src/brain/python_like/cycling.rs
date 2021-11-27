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
use crate::io::gpio::{GPIOManager, GPIOState};
use crate::io::robbable::DispatchedRobbable;

pub struct CyclingTaskHandle {
    join_handle: JoinHandle<()>,
    sender: Sender<CyclingTaskMessage>,
    sent_terminate_request: Option<Instant>,
}

impl CyclingTaskHandle {

    pub fn start_task<G>(runtime: &Runtime, gpio: DispatchedRobbable<G>, config: PythonBrainConfig, intial_sleep: Duration) -> Self
        where G: GPIOManager + Send + 'static {
        let (send, recv) = tokio::sync::mpsc::channel(10);
        let future = cycling_task(config, recv, gpio, intial_sleep);
        let handle = runtime.spawn(future);
        CyclingTaskHandle {
            join_handle: handle,
            sender: send,
            sent_terminate_request: None,
        }
    }
}

impl CyclingTaskHandle {

    pub fn join_handle(&mut self) -> &mut JoinHandle<()> {
        &mut self.join_handle
    }

    pub fn terminate_soon(&mut self, leave_on: bool) {
        if self.sent_terminate_request.is_none() {
            self.sender.try_send(CyclingTaskMessage::new(leave_on))
                .expect("Should be able to send message");
            self.sent_terminate_request = Some(Instant::now())
        }
    }

    pub fn get_sent_terminate_request(&self) -> &Option<Instant> {
        &self.sent_terminate_request
    }
}

#[derive(Debug)]
struct CyclingTaskMessage {
    leave_on: bool
}

impl CyclingTaskMessage {
    pub fn new(leave_on: bool) -> Self {
        CyclingTaskMessage {
            leave_on
        }
    }
}

async fn cycling_task<G>(config: PythonBrainConfig, mut receiver: Receiver<CyclingTaskMessage>, gpio_access: DispatchedRobbable<G>, initial_sleep_duration: Duration)
    where G: GPIOManager {

    println!("Waiting {:?} for initial sleep", initial_sleep_duration);
    if let Some(message) = wait_or_get_message(&mut receiver, initial_sleep_duration).await {
        println!("Received message during initial sleep {:?}", message);
        return;
    }

    // Turn on circulation pump.
    {
        println!("Turning on heat circulation pump since we've finished our initial sleep.");
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
            if message.leave_on {
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

    async fn wait_or_get_message(receiver: &mut Receiver<CyclingTaskMessage>, wait: Duration) -> Option<CyclingTaskMessage> {
        let result = tokio::time::timeout(wait, receiver.recv()).await;
        match result {
            Ok(None) => panic!("Other side disconnected"),
            Ok(Some(message)) => Some(message),
            Err(_timeout) => None
        }
    }
}