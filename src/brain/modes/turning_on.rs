use std::time::Instant;

use crate::brain::python_like::control::heating_control::HeatPumpMode;
use log::*;
use tokio::runtime::Runtime;

use crate::{
    brain::{python_like::config::PythonBrainConfig, BrainFailure},
    expect_available,
    io::IOBundle,
    time_util::mytime::TimeProvider,
};

use super::{intention::Intention, InfoCache, Mode, working_temp::{find_working_temp_action, CurrentHeatDirection, MixedState, WorkingTempAction}};

#[derive(Debug, PartialEq)]
pub struct TurningOnMode {
    started: Instant,
}

impl TurningOnMode {
    pub fn new(begun: Instant) -> Self {
        Self { started: begun }
    }
}

impl Mode for TurningOnMode {
    fn enter(
        &mut self,
        _config: &PythonBrainConfig,
        _runtime: &Runtime,
        io_bundle: &mut IOBundle,
    ) -> Result<(), BrainFailure> {
        let heating = expect_available!(io_bundle.heating_control())?;
        heating.set_heat_pump(HeatPumpMode::HeatingOnly, Some("Turning on HP when entering mode."))?;
        heating.set_circulation_pump(true, Some("Turning on CP when entering mode."))
    }

    fn update(
        &mut self,
        rt: &Runtime,
        config: &PythonBrainConfig,
        info_cache: &mut InfoCache,
        io_bundle: &mut IOBundle,
        time: &impl TimeProvider,
    ) -> Result<Intention, BrainFailure> {
        if !info_cache.heating_on() {
            info!("Wiser turned off before waiting time period ended");
            // TODO: Should it potentially go into overrun from this? - if not, need to switch off
            // immediately.
            return Ok(Intention::finish());
        }

        if self.started.elapsed() > config.hp_enable_time {
            return Ok(Intention::finish());
        }

        let temps = match rt.block_on(info_cache.get_temps(io_bundle.temperature_manager())) {
            Ok(t) => t,
            Err(e) => {
                error!("Failed to retrieve temperatures '{e}'");
                return Ok(Intention::KeepState)
            }
        };

        let slot = config.get_overrun_during().find_matching_slot(&time.get_utc_time(), &temps,
            |_temps, _temp| true
        );

        let heating = expect_available!(io_bundle.heating_control())?;
        match find_working_temp_action(
            &temps,
            &info_cache.get_working_temp_range(),
            &config,
            CurrentHeatDirection::None,
            Some(if heating.try_get_heat_pump()? == HeatPumpMode::BoostedHeating { MixedState::BoostedHeating } else { MixedState::NotMixed }),
            slot,
            heating.get_heat_pump_on_with_time()?.1
        ) {
            Ok((_, WorkingTempAction::Heat { mixed_state: MixedState::BoostedHeating })) => {
                heating.set_heat_pump(HeatPumpMode::BoostedHeating, Some("Enabling boost from hot water tank"))?;
            }
            _ => {
                heating.set_heat_pump(HeatPumpMode::HeatingOnly, Some("Disabling boost from hot water tank"))?;
            }
        }

        Ok(Intention::KeepState)
    }
}
