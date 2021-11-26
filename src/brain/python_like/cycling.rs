use std::fs::read;
use std::ops::Add;
use std::sync::mpsc;
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::thread::sleep;
use std::time::{Duration, Instant};
use chrono::{DateTime, Local, SecondsFormat, Utc};
use tokio::runtime::Runtime;
use tokio::task::JoinHandle;
use crate::brain::python_like::{HEAT_PUMP_RELAY, PythonBrainConfig};
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
        let (send, recv) = mpsc::channel();
        let future = cycling_task(config, recv, gpio, false, intial_sleep);
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
            self.sender.send(CyclingTaskMessage::new(leave_on))
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

const INITIAL_SLEEP_BLOCK_SECONDS: u64 = 60;

fn format_datetime(datetime: &DateTime<Utc>) -> String {
    return format!("{}", datetime.with_timezone(&Local).to_rfc3339_opts(SecondsFormat::Secs, true));
}

async fn cycling_task<G>(config: PythonBrainConfig, receiver: Receiver<CyclingTaskMessage>, mut gpio_access: DispatchedRobbable<G>, first_on: bool, initial_sleep_duration: Duration)
    where G: GPIOManager {

    {
        let end_initial_sleep = Utc::now() + chrono::Duration::from_std(initial_sleep_duration).unwrap();
        println!("Initial sleep until {}", format_datetime(&end_initial_sleep));
        loop {
            // Check messages.
            let latest_message = read_latest_message(&receiver);
            if latest_message.is_err() {
                println!("Latest message was an error, not sure whats going on {}", latest_message.unwrap_err());
                return;
            }
            if let Ok(Some(message)) = latest_message {
                println!("Received stop message during initial sleep, stopping.");
                return;
            }
            let now = Utc::now();
            if now > end_initial_sleep {
                println!("Finished initial sleep");
                break;
            }
            let remaining = end_initial_sleep - now;

            if remaining.num_seconds() > (INITIAL_SLEEP_BLOCK_SECONDS as i64) {
                println!("Sleeping {} seconds before re-checking messages. Initial sleep ends at {}", INITIAL_SLEEP_BLOCK_SECONDS, format_datetime(&end_initial_sleep));
                tokio::time::sleep(Duration::from_secs(INITIAL_SLEEP_BLOCK_SECONDS)).await;
            }
            else {
                println!("Sleeping {} seconds and then the initial sleep will be complete.", remaining.num_seconds());
                tokio::time::sleep(remaining.to_std().unwrap()).await;
            }
        }
    }
    let mut next_state_on = !first_on;
    let mut sleep_length;
    loop {
        sleep_length = get_sleep_length(next_state_on, &config);
        println!("Will change to {}. Sleeping for {:?}", next_state_on, sleep_length);
        tokio::time::sleep(sleep_length).await;
        let latest_message = read_latest_message(&receiver);
        if let Ok(Some(message)) = &latest_message {
            println!("Received Message {:?}", message);
            if message.leave_on {
                // If we are leaving it on (if possible) then it doesn't matter about the current state.
                break;
            }
            // Otherwise, turn off
            let mut lock_result = gpio_access.access().lock().expect("Mutex on gpio is poisoned");
            if lock_result.is_none() {
                println!("Cycling Task - We no longer have the gpio, someone probably robbed it.");
                return;
            }
            let mut gpio = lock_result.as_mut().unwrap();
            gpio.set_pin(HEAT_PUMP_RELAY, &GPIOState::HIGH)
                .expect("Should be able to set Heat Pump Relay to High");
            break;
        }
        let mut lock_result = gpio_access.access().lock().expect("Mutex on gpio is poisoned");
        if lock_result.is_none() {
            println!("Cycling Task - We no longer have the gpio, someone probably robbed it.");
            return;
        }
        let mut gpio = lock_result.as_mut().unwrap();
        if next_state_on {
            if latest_message.is_err() {
                // Safely terminate.
                break;
            }
            // TODO: Should we be expecting here?
            gpio.set_pin(HEAT_PUMP_RELAY, &GPIOState::LOW)
                .expect("Should be able to set Heat Pump Relay to Low");
        }
        else {
            gpio.set_pin(HEAT_PUMP_RELAY, &GPIOState::HIGH)
                .expect("Should be able to set Heat Pump Relay to Low");
            if latest_message.is_err() {
                // Now safe since we set the pin.
                break;
            }
        }
        next_state_on = !next_state_on;
    }
}

fn get_sleep_length(next_state_on: bool, config: &PythonBrainConfig) -> Duration {
    if next_state_on {
        config.hp_pump_off_time
    }
    else {
        config.hp_pump_on_time
    }
}

fn read_latest_message<T>(receiver: &Receiver<T>) -> Result<Option<T>, String> {
    let mut message = None;
    loop {
        match receiver.try_recv() {
            Ok(ok) => message = Some(ok),
            Err(err) => match err {
                TryRecvError::Empty => break,
                TryRecvError::Disconnected => return Err("Other end Disconnected!".to_owned()),
            }
        }
    }
    Ok(message)
}