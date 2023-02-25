use core::option::Option;
use core::option::Option::{None, Some};
use std::time::{Duration, Instant};
use futures::FutureExt;
use log::{error, info, warn};
use tokio::runtime::Runtime;
use tokio::sync::mpsc::Sender;
use tokio::task::JoinHandle;
use crate::python_like::modes::heating_mode::SharedData;
use crate::python_like::modes::{InfoCache, Intention, Mode};
use crate::{brain_fail, BrainFailure, CorrectiveActions, IOBundle, PythonBrainConfig, Sensor};
use crate::python_like::cycling;
use crate::time::mytime::TimeProvider;

#[derive(Debug)]
pub enum CirculateStatus {
    Uninitialised,
    Active(CirculateHeatPumpOnlyTaskHandle),
    Stopping(StoppingStatus)
}

impl Mode for CirculateStatus {
    fn update(&mut self, _shared_data: &mut SharedData, rt: &Runtime, config: &PythonBrainConfig, info_cache: &mut InfoCache, io_bundle: &mut IOBundle, _time: &impl TimeProvider) -> Result<Intention, BrainFailure> {
        match self {
            CirculateStatus::Uninitialised => {
                if !info_cache.heating_on() {
                    return Ok(Intention::finish());
                }

                let dispatched_gpio = io_bundle.dispatch_heating_control()
                    .map_err(|_| brain_fail!("Failed to dispatch gpio into circulation task", CorrectiveActions::unknown_heating()))?;
                let task = cycling::start_task(rt, dispatched_gpio, config.get_hp_circulation_config().clone());
                *self = CirculateStatus::Active(task);
                warn!("Had to initialise CirculateStatus during update.");
                return Ok(Intention::KeepState);
            }
            CirculateStatus::Active(_) => {
                let mut stop_cycling = || {
                    let old_status = std::mem::replace(self, CirculateStatus::Uninitialised);
                    if let CirculateStatus::Active(active) = old_status {
                        *self = CirculateStatus::Stopping(active.terminate_soon(false));
                        Ok(())
                    } else {
                        return Err(brain_fail!("We just checked and it was active, so it should still be!", CorrectiveActions::unknown_heating()));
                    }
                };

                if !info_cache.heating_on() {
                    stop_cycling()?;
                    return Ok(Intention::KeepState);
                }

                let temps = rt.block_on(info_cache.get_temps(io_bundle.temperature_manager()));
                if let Err(err) = temps {
                    error!("Failed to retrieve temperatures {}. Stopping cycling.", err);
                    stop_cycling()?;
                    return Ok(Intention::KeepState);
                }
                let temps = temps.unwrap();

                if let Some(temp) = temps.get(&Sensor::TKBT) {
                    info!(target: "cycling_watch", "TKBT: {:.2}", temp);
                    let working_range = info_cache.get_working_temp_range();
                    if *temp < working_range.get_min() {
                        stop_cycling()?;
                        return Ok(Intention::KeepState);
                    }
                }
            }
            CirculateStatus::Stopping(status) => {
                if status.check_ready() {
                    // Retrieve heating control so other states can use it.
                    io_bundle.heating_control().rob_or_get_now()
                        .map_err(|_| brain_fail!("Couldn't retrieve control of gpio after cycling (in stopping update)", CorrectiveActions::unknown_heating()))?;
                    return Ok(Intention::finish());
                } else if status.sent_terminate_request_time().elapsed() > Duration::from_secs(2) {
                    return Err(brain_fail!(format!("Didn't get back gpio from cycling task (Elapsed: {:?})", status.sent_terminate_request_time().elapsed()), CorrectiveActions::unknown_heating()));
                }
            }
        }
        Ok(Intention::KeepState)
    }
}

#[derive(Debug)]
pub struct StoppingStatus {
    join_handle: Option<JoinHandle<()>>,
    sender: Sender<CirculateHeatPumpOnlyTaskMessage>,
    sent_terminate_request: Instant,
}

impl StoppingStatus {

    pub fn stopped() -> Self {
        let (tx, _) = tokio::sync::mpsc::channel(1);
        Self {
            join_handle: None,
            sender: tx,
            sent_terminate_request: Instant::now(),
        }
    }

    pub fn sent_terminate_request_time(&self) -> &Instant {
        &self.sent_terminate_request
    }

    pub fn check_ready(&mut self) -> bool {
        match &mut self.join_handle {
            None => true,
            Some(handle) => {
                if handle.now_or_never().is_some() {
                    self.join_handle.take();
                    return true;
                }
                false
            },
        }
    }
}

#[derive(Debug)]
pub struct CirculateHeatPumpOnlyTaskHandle {
    join_handle: JoinHandle<()>,
    sender: Sender<CirculateHeatPumpOnlyTaskMessage>,
}

impl CirculateHeatPumpOnlyTaskHandle {

    pub fn new(join_handle: JoinHandle<()>, sender: Sender<CirculateHeatPumpOnlyTaskMessage>) -> Self {
        CirculateHeatPumpOnlyTaskHandle {
            join_handle,
            sender,
        }
    }

    pub fn join_handle(&mut self) -> &mut JoinHandle<()> {
        &mut self.join_handle
    }

    pub fn terminate_soon(self, leave_on: bool) -> StoppingStatus {
        self.sender.try_send(CirculateHeatPumpOnlyTaskMessage::new(leave_on))
            .expect("Should be able to send message");

        StoppingStatus {
            join_handle: Some(self.join_handle),
            sender: self.sender,
            sent_terminate_request: Instant::now(),
//            ready: false
        }
    }
}

#[derive(Debug)]
pub struct CirculateHeatPumpOnlyTaskMessage {
    leave_on: bool
}

impl CirculateHeatPumpOnlyTaskMessage {
    pub fn new(leave_on: bool) -> Self {
        CirculateHeatPumpOnlyTaskMessage {
            leave_on
        }
    }

    pub fn leave_on(&self) -> bool {
        self.leave_on
    }
}
