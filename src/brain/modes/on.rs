use crate::brain::modes::heating_mode::HeatingMode;
use crate::brain::modes::intention::Intention;
use crate::brain::modes::{InfoCache, Mode};
use crate::brain::python_like::config::PythonBrainConfig;
use crate::brain::python_like::control::heating_control::HeatPumpMode;
use crate::brain::BrainFailure;
use crate::expect_available;
use crate::io::temperatures::Sensor;
use crate::io::IOBundle;
use crate::time_util::mytime::TimeProvider;
use log::*;
use tokio::runtime::Runtime;

use super::working_temp::{find_working_temp_action, CurrentHeatDirection, WorkingTempAction, MixedState};

#[derive(Debug, PartialEq)]
pub struct OnMode {
    circulation_pump_on: bool,
}

impl OnMode {
    pub fn new(circulation_pump_on: bool) -> Self {
        Self { circulation_pump_on }
    }
}

impl Default for OnMode {
    fn default() -> Self {
        Self::new(false)
    }
}

/// This state also covers boosted heating
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
                heating.set_heat_pump(HeatPumpMode::HeatingOnly, Some("Turning on HP when entering mode"))?;
            }
        }

        let cp = heating.get_circulation_pump()?.0;
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
            // Finish mode should pick up any overrun whether considering
            // minimum run time or not.
            // TODO: config.get_min_hp_runtime();
            // min_runtime.get_safety_cut_off().get_target_sensor().clone(),
            return Ok(Intention::finish());
        }

        let slot = config.get_overrun_during().find_best_slot(false, time.get_utc_time(), &temps,
            Some(""),
            |_temps, _temp| true
        );

        let heating = expect_available!(io_bundle.heating_control())?;

        match find_working_temp_action(
            &temps,
            &info_cache.get_working_temp_range(),
            config,
            CurrentHeatDirection::Climbing,
            Some(if heating.try_get_heat_pump()? == HeatPumpMode::BoostedHeating { MixedState::BoostedHeating } else { MixedState::NotMixed }),
            slot,
            heating.get_heat_pump_on_with_time()?.1
        ) {
            Ok((Some(heating_mode @ HeatingMode::DhwOnly(_)), _)) => {
                return Ok(Intention::SwitchForce(heating_mode));
            }
            Ok((_, WorkingTempAction::Heat { mixed_state: MixedState::MixedHeating })) => {
                /* TODO
                let on_duration = heating.get_heat_pump_on_with_time()?.1;
                if on_duration > Duration::from_mins(40) {
                    debug!("Would consider mixed mode, but heat pump has been on for {on_duration:?}, so switching off"); 
                    return Ok(Intention::off_now());
                    //return Ok(Intention::SwitchForce(HeatingMode::Off(OffMode::default())));
                }
                */
                // TODO: Go straight to MixedMode
                debug!("Finishing On mode to check for Mixed mode. - Legacy 2");
                return Ok(Intention::finish());
            }
            Ok((_, WorkingTempAction::Heat { mixed_state: MixedState::NotMixed })) => {               
                heating.set_heat_pump(HeatPumpMode::HeatingOnly, Some("Disabling boost from hot water tank"))?;
            }
            Ok((_, WorkingTempAction::Heat { mixed_state: MixedState::BoostedHeating })) => {
                heating.set_heat_pump(HeatPumpMode::BoostedHeating, Some("Enabling boost from hot water tank"))?;
            }
            Ok((_, WorkingTempAction::Cool { .. })) => {
                info!("Hit top of working range - should no longer heat");
                return Ok(Intention::finish());
            }
            Err(missing_sensor) => {
                error!("Can't check whether to circulate due to missing sensor: {missing_sensor}");
                return Ok(Intention::off_now());
            }
        }

        // Stay in Heating On

        if !self.circulation_pump_on {
            if let Some(temp) = temps.get(&Sensor::HPRT) {
                if *temp > config.temp_before_circulate {
                    info!("Reached min circulation temp.");
                    heating.try_set_circulation_pump(true)?;
                    self.circulation_pump_on = true;
                }
            }
        }

        Ok(Intention::YieldHeatUps)
    }
}
