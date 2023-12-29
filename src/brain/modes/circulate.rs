use crate::brain::modes::heating_mode::SharedData;
use crate::brain::modes::{InfoCache, Intention, Mode};
use crate::brain::python_like::config::heat_pump_circulation::HeatPumpCirculationConfig;
use crate::brain::python_like::working_temp::WorkingRange;
use crate::io::temperatures::Sensor;
use crate::python_like::cycling;
use crate::time_util::mytime::TimeProvider;
use crate::{brain_fail, BrainFailure, CorrectiveActions, IOBundle, PythonBrainConfig};
use core::option::Option;
use core::option::Option::{None, Some};
use futures::FutureExt;
use log::{error, info, warn};
use std::time::{Duration, Instant};
use tokio::runtime::Runtime;
use tokio::sync::mpsc::Sender;
use tokio::task::JoinHandle;

use super::heating_mode::PossibleTemperatureContainer;

#[derive(Debug)]
pub enum CirculateStatus {
    Uninitialised,
    Active(CirculateHeatPumpOnlyTaskHandle),
    Stopping(StoppingStatus),
}

impl PartialEq for CirculateStatus {
    fn eq(&self, _: &Self) -> bool {
        false
    }
}

impl Mode for CirculateStatus {
    fn enter(
        &mut self,
        config: &PythonBrainConfig,
        runtime: &Runtime,
        io_bundle: &mut IOBundle,
    ) -> Result<(), BrainFailure> {
        // Dispatch to separate thread.
        if let CirculateStatus::Uninitialised = &self {
            let dispatched_gpio = io_bundle.dispatch_heating_control().map_err(|_| {
                brain_fail!(
                    "Failed to dispatch gpio into circulation task",
                    CorrectiveActions::unknown_heating()
                )
            })?;
            let task = cycling::start_task(
                runtime,
                dispatched_gpio,
                config.get_hp_circulation_config().clone(),
            );
            *self = CirculateStatus::Active(task);
        }
        Ok(())
    }

    fn update(
        &mut self,
        _shared_data: &mut SharedData,
        rt: &Runtime,
        config: &PythonBrainConfig,
        info_cache: &mut InfoCache,
        io_bundle: &mut IOBundle,
        _time: &impl TimeProvider,
    ) -> Result<Intention, BrainFailure> {
        match self {
            CirculateStatus::Uninitialised => {
                if !info_cache.heating_on() {
                    return Ok(Intention::finish());
                }

                let dispatched_gpio = io_bundle.dispatch_heating_control().map_err(|_| {
                    brain_fail!(
                        "Failed to dispatch gpio into circulation task",
                        CorrectiveActions::unknown_heating()
                    )
                })?;
                let task = cycling::start_task(
                    rt,
                    dispatched_gpio,
                    config.get_hp_circulation_config().clone(),
                );
                *self = CirculateStatus::Active(task);
                warn!("Had to initialise CirculateStatus during update.");
                return Ok(Intention::KeepState);
            }
            CirculateStatus::Active(_) => {
                // TODO: Stop cycling should leave on if we would go into any mode other than off pretty much.
                let mut stop_cycling = |leave_on| {
                    let old_status = std::mem::replace(self, CirculateStatus::Uninitialised);
                    if let CirculateStatus::Active(active) = old_status {
                        *self = CirculateStatus::Stopping(active.terminate_soon(leave_on));
                        Ok(())
                    } else {
                        Err(brain_fail!(
                            "We just checked and it was active, so it should still be!",
                            CorrectiveActions::unknown_heating()
                        ))
                    }
                };

                if !info_cache.heating_on() {
                    stop_cycling(false)?;
                    return Ok(Intention::KeepState);
                }

                let temps = rt.block_on(info_cache.get_temps(io_bundle.temperature_manager()));
                if let Err(err) = temps {
                    error!("Failed to retrieve temperatures {}. Stopping cycling.", err);
                    stop_cycling(false)?;
                    return Ok(Intention::KeepState);
                }
                match should_circulate_using_forecast(
                    &temps.unwrap(),
                    &info_cache.get_working_temp_range(),
                    config.get_hp_circulation_config(),
                    CurrentHeatDirection::Falling,
                ) {
                    Ok(true) => {}
                    Ok(false) => {
                        info!("Hit bottom of working range, stopping cycling.");
                        stop_cycling(true)?;
                    }
                    Err(missing_sensor) => {
                        error!("Unable to check whether to exit circulation due to missing sensor: {}. Turning off.", missing_sensor);
                        stop_cycling(false)?;
                    }
                };
                return Ok(Intention::KeepState);
            }
            CirculateStatus::Stopping(status) => {
                if status.check_ready() {
                    // Retrieve heating control so other states can use it.
                    io_bundle.heating_control().rob_or_get_now().map_err(|_| {
                        brain_fail!(
                            "Couldn't retrieve control of gpio after cycling (in stopping update)",
                            CorrectiveActions::unknown_heating()
                        )
                    })?;
                    return Ok(Intention::finish());
                } else if status.sent_terminate_request_time().elapsed() > Duration::from_secs(2) {
                    return Err(brain_fail!(
                        format!(
                            "Didn't get back gpio from cycling task (Elapsed: {:?})",
                            status.sent_terminate_request_time().elapsed()
                        ),
                        CorrectiveActions::unknown_heating()
                    ));
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
            }
        }
    }
}

#[derive(Debug)]
pub struct CirculateHeatPumpOnlyTaskHandle {
    join_handle: JoinHandle<()>,
    sender: Sender<CirculateHeatPumpOnlyTaskMessage>,
}

impl CirculateHeatPumpOnlyTaskHandle {
    pub fn new(
        join_handle: JoinHandle<()>,
        sender: Sender<CirculateHeatPumpOnlyTaskMessage>,
    ) -> Self {
        CirculateHeatPumpOnlyTaskHandle {
            join_handle,
            sender,
        }
    }

    pub fn terminate_soon(self, leave_on: bool) -> StoppingStatus {
        self.sender
            .try_send(CirculateHeatPumpOnlyTaskMessage::new(leave_on))
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
    leave_on: bool,
}

impl CirculateHeatPumpOnlyTaskMessage {
    pub fn new(leave_on: bool) -> Self {
        CirculateHeatPumpOnlyTaskMessage { leave_on }
    }

    pub fn leave_on(&self) -> bool {
        self.leave_on
    }
}

/// Which way we are currently travelling within the working range.
pub enum CurrentHeatDirection {
    /// Just started up. Fine to go either up or down.
    None,
    /// Already climbing / temperature rising, only start circulating once we hit the top.
    Climbing,
    /// Already falling (circulating already), only stop circulating once we hit the bottom.
    Falling,
}

/// Forecasts what the TKBT is likely to be soon based on the temperature of HXOR since
/// it will drop quickly if HXOR is low (and hence maybe we should go straight to On).
/// Returns the forecasted temperature, or the sensor that was missing.
pub fn should_circulate_using_forecast(
    temps: &impl PossibleTemperatureContainer,
    range: &WorkingRange,
    config: &HeatPumpCirculationConfig,
    current_circulate_state: CurrentHeatDirection,
) -> Result<bool, Sensor> {
    let tkbt = temps.get_sensor_temp(&Sensor::TKBT).ok_or(Sensor::TKBT)?;
    let hxor = temps.get_sensor_temp(&Sensor::HXOR).ok_or(Sensor::HXOR)?;

    let additional = (tkbt - hxor - config.get_forecast_diff_offset()).clamp(0.0, 20.0)
        * config.get_forecast_diff_proportion();
    let adjusted_temp = (tkbt - additional).clamp(0.0, crate::python_like::MAX_ALLOWED_TEMPERATURE);

    let range_width = range.get_max() - range.get_min();

    let pct = (adjusted_temp - range.get_min()) / range_width;

    let info_msg = if pct > 1.0 {
        "Above top".to_owned()
    } else if pct < 0.0 {
        "Below bottom".to_owned()
    } else {
        match current_circulate_state {
            CurrentHeatDirection::None => format!(
                "{:.0}%, initial req. {:.0}%",
                pct * 100.0,
                config.get_forecast_start_above_percent() * 100.0
            ),
            _ => format!("{:.0}%", pct * 100.0),
        }
    };

    info!(
        "TKBT: {:.2}, HXOR: {:.2}, Forecast temp: {:.2} ({})",
        tkbt, hxor, adjusted_temp, info_msg,
    );

    Ok(match current_circulate_state {
        CurrentHeatDirection::Falling => pct >= 0.0,
        CurrentHeatDirection::Climbing => pct >= 1.0,
        CurrentHeatDirection::None => pct > config.get_forecast_start_above_percent(),
    })
}
