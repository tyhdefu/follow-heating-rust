use crate::brain::modes::{InfoCache, Intention, Mode};
use crate::brain::python_like::control::heating_control::HeatPumpMode;
use crate::time_util::mytime::TimeProvider;
use crate::{expect_available, BrainFailure, IOBundle, PythonBrainConfig};
use core::option::Option::{None, Some};
use log::{error, info};
use tokio::runtime::Runtime;

use super::working_temp::{find_working_temp_action, CurrentHeatDirection, WorkingTempAction};

#[derive(Debug, PartialEq, Default)]
pub struct CirculateMode {}

impl Mode for CirculateMode {
    fn enter(
        &mut self,
        _config: &PythonBrainConfig,
        _runtime: &Runtime,
        io_bundle: &mut IOBundle,
    ) -> Result<(), BrainFailure> {
        let heating = expect_available!(io_bundle.heating_control())?;
        if heating.try_get_heat_pump()? != HeatPumpMode::DrainTank {
            heating.try_set_heat_pump(HeatPumpMode::DrainTank)?;
        }
        if !heating.try_get_heat_circulation_pump()? {
            heating.try_set_heat_circulation_pump(true)?;
        }
        Ok(())
    }

    fn update(
        &mut self,
        rt: &Runtime,
        config: &PythonBrainConfig,
        info_cache: &mut InfoCache,
        io_bundle: &mut IOBundle,
        _time: &impl TimeProvider,
    ) -> Result<Intention, BrainFailure> {
        if !info_cache.heating_on() {
            return Ok(Intention::finish());
        }
        let temps = match rt.block_on(info_cache.get_temps(io_bundle.temperature_manager())) {
            Ok(temps) => temps,
            Err(e) => {
                error!("Failed to retrieve temperatures: {} - Turning off.", e);
                return Ok(Intention::off_now());
            }
        };
        let range = info_cache.get_working_temp_range();
        match find_working_temp_action(
            &temps,
            &range,
            config.get_hp_circulation_config(),
            CurrentHeatDirection::Falling,
        ) {
            Ok(WorkingTempAction::Cool { circulate: true }) => Ok(Intention::YieldHeatUps),
            Ok(WorkingTempAction::Cool { circulate: false }) => {
                info!("TKBT too cold, would be heating the tank. ending circulation.");
                Ok(Intention::finish())
            }
            Ok(WorkingTempAction::Heat { allow_mixed: _ }) => {
                info!("Reached bottom of working range, ending circulation.");
                Ok(Intention::Finish)
            }
            Err(missing_sensor) => {
                error!(
                    "Could not check whether to circulate due to missing sensor: {} - turning off.",
                    missing_sensor
                );
                Ok(Intention::off_now())
            }
        }
    }
}
