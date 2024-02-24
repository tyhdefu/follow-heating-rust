use std::time::Instant;

use crate::brain::modes::dhw_only::DhwOnlyMode;
use crate::brain::modes::heating_mode::HeatingMode;
use crate::brain::modes::intention::Intention;
use crate::brain::modes::{InfoCache, Mode};
use crate::brain::python_like::config::PythonBrainConfig;
use crate::brain::python_like::config::overrun_config::DhwTemps;
use crate::brain::python_like::control::heating_control::HeatPumpMode;
use crate::brain::BrainFailure;
use crate::expect_available;
use crate::io::temperatures::Sensor;
use crate::io::IOBundle;
use crate::time_util::mytime::TimeProvider;
use log::{debug, error, info, warn};
use tokio::runtime::Runtime;

use super::working_temp::{find_working_temp_action, CurrentHeatDirection, WorkingTempAction, MixedState};

#[derive(Debug, PartialEq)]
pub struct OnMode {
    circulation_pump_on: bool,

    // TODO: This is one of the root causes of reported On => On transitions when it
    // looks like it might be able to go into MixedMode. Also it seem less than
    // foolproof is it needs to be carefully retained between states (which it isn't,
    // as those falsely reported transitions demontrate).
    // It would probably be better to have values for "last on" and "last off" state
    // in IoBundle with a function that determines whether the HP is actually on and
    // how long it has been on for.
    started: Instant,
}

impl OnMode {
    pub fn create(circulation_pump_on: bool) -> Self {
        Self {
            circulation_pump_on,
            started: Instant::now(),
        }
    }

    pub fn new(circulation_pump_on: bool, started: Instant) -> Self {
        Self {
            circulation_pump_on, started,
        }
    }
}

impl Default for OnMode {
    fn default() -> Self {
        Self::create(false)
    }
}

impl Mode for OnMode {
    fn enter(
        &mut self,
        _config: &PythonBrainConfig,
        _runtime: &Runtime,
        io_bundle: &mut IOBundle,
    ) -> Result<(), BrainFailure> {
        let heating = expect_available!(io_bundle.heating_control())?;

        match heating.try_get_heat_pump()? {
            HeatPumpMode::HeatingOnly | HeatPumpMode::BoostedHeating => {},
            _ => {
                debug!("Turning on HP when entering mode.");
                heating.try_set_heat_pump(HeatPumpMode::HeatingOnly)?;
            }
        }

        let cp = heating.try_get_heat_circulation_pump()?;
        if self.circulation_pump_on != cp {
            debug!("Setting internal circulation pump on to {}", cp);
            self.circulation_pump_on = cp;
        }
        Ok(())
    }

    fn update(
        &mut self,
        rt: &Runtime,
        config: &PythonBrainConfig,
        info_cache: &mut InfoCache,
        io_bundle: &mut IOBundle,
        time: &impl TimeProvider,
    ) -> Result<Intention, BrainFailure> {
        let temps = rt.block_on(info_cache.get_temps(io_bundle.temperature_manager()));
        if let Err(err) = temps {
            error!("Failed to retrieve temperatures {}. Turning off.", err);
            return Ok(Intention::off_now());
        }
        let temps = temps.unwrap();

        if !info_cache.heating_on() {
            // TODO: 6 minute / overrun should move to Intention / tracking out of state.
            let running_for = self.started.elapsed();
            let min_runtime = config.get_min_hp_runtime();
            if running_for < *min_runtime.get_min_runtime() {
                warn!(
                    "Warning: Carrying on until the {} second mark or safety cut off: {}",
                    min_runtime.get_min_runtime().as_secs(),
                    min_runtime.get_safety_cut_off()
                );
                let remaining = *min_runtime.get_min_runtime() - running_for;
                let end = time.get_utc_time() + chrono::Duration::from_std(remaining).unwrap();
                return Ok(Intention::SwitchForce(HeatingMode::DhwOnly(
                    DhwOnlyMode::from_time(
                        DhwTemps {
                            sensor: min_runtime.get_safety_cut_off().get_target_sensor().clone(),
                            min: 0.0,
                            max: min_runtime.get_safety_cut_off().get_target_temp(),
                            extra: None,
                        },
                        end
                    )
                )));
            }
            return Ok(Intention::finish());
        }

        let heating = expect_available!(io_bundle.heating_control())?;
        match find_working_temp_action(
            &temps,
            &info_cache.get_working_temp_range(),
            config.get_hp_circulation_config(),
            CurrentHeatDirection::Climbing,
            Some(if heating.try_get_heat_pump()? == HeatPumpMode::BoostedHeating { MixedState::BoostedHeating } else { MixedState::NotMixed }),
        ) {
            Ok(WorkingTempAction::Heat { mixed_state: MixedState::MixedHeating }) => {
                debug!("Finishing On mode to check for mixed mode.");
                return Ok(Intention::finish());
            }
            Ok(WorkingTempAction::Heat { mixed_state: MixedState::NotMixed }) => {               
                heating.set_heat_pump(HeatPumpMode::HeatingOnly, Some("Disabling boost from hot water tank"))?;
            }
            Ok(WorkingTempAction::Heat { mixed_state: MixedState::BoostedHeating }) => {
                heating.set_heat_pump(HeatPumpMode::BoostedHeating, Some("Enabling boost from hot water tank"))?;
            }
            Ok(WorkingTempAction::Cool { .. }) => {
                info!("Hit top of working range - should no longer heat");
                return Ok(Intention::finish());
            }
            Err(missing_sensor) => {
                error!(
                    "Can't check whether to circulate due to missing sensor: {}",
                    missing_sensor
                );
                return Ok(Intention::off_now());
            }
        }
        if !self.circulation_pump_on {
            if let Some(temp) = temps.get(&Sensor::HPRT) {
                if *temp > config.get_temp_before_circulate() {
                    info!("Reached min circulation temp.");
                    let gpio = expect_available!(io_bundle.heating_control())?;
                    gpio.try_set_heat_circulation_pump(true)?;
                    self.circulation_pump_on = true;
                }
            }
        }
        Ok(Intention::YieldHeatUps)
    }
}
