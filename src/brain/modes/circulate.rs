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
        heating.set_heat_pump(HeatPumpMode::DrainTank, None)?;
        heating.set_heat_circulation_pump(true, None)
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
            &config,
            CurrentHeatDirection::Falling,
            None, None,
            expect_available!(io_bundle.heating_control())?.as_hp().get_heat_pump_on_with_time()?.1
        ) {
            Ok((_, WorkingTempAction::Cool { circulate: true })) => Ok(Intention::YieldHeatUps),
            Ok((_, WorkingTempAction::Cool { circulate: false })) => {
                info!("TKBT too cold, would be heating the tank. Ending circulation.");
                Ok(Intention::finish())
            }
            Ok((_, WorkingTempAction::Heat { .. })) => {
                info!("Reached bottom of working range, ending circulation.");
                Ok(Intention::Finish)
            }
            Err(missing_sensor) => {
                error!("Could not check whether to circulate due to missing sensor: {missing_sensor} - turning off.");
                Ok(Intention::off_now())
            }
        }
    }
}
