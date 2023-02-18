use std::time::Duration;
use tokio::runtime::Runtime;
use tokio::sync::mpsc::Receiver;
use crate::brain::python_like::modes::circulate::{CirculateHeatPumpOnlyTaskHandle, CirculateHeatPumpOnlyTaskMessage};
use crate::HeatingControl;
use crate::io::robbable::DispatchedRobbable;
use crate::brain::python_like::config::heat_pump_circulation::HeatPumpCirculationConfig;

pub fn start_task(runtime: &Runtime, gpio: DispatchedRobbable<Box<dyn HeatingControl>>, config: HeatPumpCirculationConfig) -> CirculateHeatPumpOnlyTaskHandle {
    let (send, recv) = tokio::sync::mpsc::channel(10);
    let future = cycling_task(config, recv, gpio);
    let handle = runtime.spawn(future);
    CirculateHeatPumpOnlyTaskHandle::new(handle, send)
}
// 1 minute 20 seconds until it will turn on.
async fn cycling_task(config: HeatPumpCirculationConfig, mut receiver: Receiver<CirculateHeatPumpOnlyTaskMessage>, heating_control_access: DispatchedRobbable<Box<dyn HeatingControl>>) {

    // Turn on circulation pump.
    {
        println!("Turning on heat circulation pump");
        let mut lock_result = heating_control_access.access().lock().expect("Mutex on gpio is poisoned");
        if lock_result.is_none() {
            println!("Cycling Task - We no longer have the gpio, someone probably robbed it.");
            return;
        }
        let gpio = lock_result.as_mut().unwrap();
        gpio.try_set_heat_circulation_pump(true)
            .expect("Should be able to set Heat Pump Relay to High");
    }

    let heat_circulation_pump_wait = Duration::from_secs(15);
    println!("Leaving heat circulation pump on for {} seconds before continuing", heat_circulation_pump_wait.as_secs());
    if let Some(message) = wait_or_get_message(&mut receiver, heat_circulation_pump_wait).await {
        println!("Received message during second part of sleep {:?}", message);
        return;
    }

    println!("Starting the main cycling task loop.");
    loop {
        // Turn on gpio.
        println!("Turning on heat pump.");
        set_heat_pump_state(&heating_control_access, true);

        println!("Waiting {:?}", config.get_hp_on_time());
        if let Some(message) = wait_or_get_message(&mut receiver, config.get_hp_on_time().clone()).await {
            println!("Received message during while on {:?}", message);
            if message.leave_on() {
                // Do nothing.
            }
            else {
                set_heat_pump_state(&heating_control_access, false);
            }
            return;
        }

        println!("Turning off heat pump");
        set_heat_pump_state(&heating_control_access, false);

        println!("Waiting {:?}", config.get_hp_off_time());
        if let Some(message) = wait_or_get_message(&mut receiver, config.get_hp_off_time().clone()).await {
            println!("Received message during while off {:?}", message);
            return;
        }
    }

    fn set_heat_pump_state(robbable: &DispatchedRobbable<Box<dyn HeatingControl>>, on: bool) {
        let mut lock_result = robbable.access().lock().expect("Mutex on gpio is poisoned");
        if lock_result.is_none() {
            println!("Cycling Task - We no longer have the gpio, someone probably robbed it.");
            return;
        }
        let gpio = lock_result.as_mut().unwrap();
        gpio.try_set_heat_pump(on)
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