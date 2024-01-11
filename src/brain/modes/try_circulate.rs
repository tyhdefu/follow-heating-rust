use std::time::Instant;

use log::{debug, error, info};
use tokio::runtime::Runtime;

use crate::brain::modes::circulate::CirculateMode;
use crate::brain::modes::heating_mode::HeatingMode;
use crate::brain::modes::turning_on::TurningOnMode;
use crate::brain::python_like::config::PythonBrainConfig;
use crate::brain::BrainFailure;
use crate::expect_available;
use crate::io::IOBundle;
use crate::python_like::control::heating_control::HeatPumpMode;
use crate::time_util::mytime::TimeProvider;

use super::intention::Intention;
use super::working_temp::{find_working_temp_action, CurrentHeatDirection, WorkingTempAction};
use super::{InfoCache, Mode};

#[derive(Debug, PartialEq)]
pub struct TryCirculateMode {
    started: Instant,
}

impl TryCirculateMode {
    pub fn new(started: Instant) -> Self {
        Self { started }
    }

    pub fn start() -> Self {
        Self {
            started: Instant::now(),
        }
    }
}

impl Mode for TryCirculateMode {
    fn enter(
        &mut self,
        config: &PythonBrainConfig,
        _runtime: &Runtime,
        io_bundle: &mut IOBundle,
    ) -> Result<(), BrainFailure> {
        info!(
            "Turning on tank circulation for {}s to see how it goes.",
            config
                .get_hp_circulation_config()
                .sample_tank_time()
                .as_secs()
        );
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
            return Ok(Intention::Finish);
        }

        let temps = match rt.block_on(info_cache.get_temps(io_bundle.temperature_manager())) {
            Ok(temps) => temps,
            Err(e) => {
                error!(
                    "Failed to retrieve temperatures in TryCirculate: {}, turning off.",
                    e
                );
                return Ok(Intention::off_now());
            }
        };

        if &self.started.elapsed() > config.get_hp_circulation_config().sample_tank_time() {
            return match find_working_temp_action(
                &temps,
                &info_cache.get_working_temp_range(),
                config.get_hp_circulation_config(),
                CurrentHeatDirection::Falling,
            ) {
                Ok(WorkingTempAction::Heat { allow_mixed: _ }) => {
                    info!("End of try period, heating is recommended.");
                    Ok(Intention::SwitchForce(HeatingMode::TurningOn(
                        TurningOnMode::new(Instant::now()),
                    )))
                }
                Ok(WorkingTempAction::Cool { circulate: true }) => {
                    info!("End of try period, deciding to circulate");
                    Ok(Intention::SwitchForce(HeatingMode::Circulate(
                        CirculateMode::default(),
                    )))
                }
                Ok(WorkingTempAction::Cool { circulate: false }) => {
                    info!("End of try period, want to cool but not circulate. Finishing mode.");
                    Ok(Intention::Finish)
                }
                Err(missing_sensor) => {
                    error!(
                        "Missing {} sensor to decide whether to circulate, stopping",
                        missing_sensor
                    );
                    Ok(Intention::Finish)
                }
            };
        }

        match find_working_temp_action(
            &temps,
            &info_cache.get_working_temp_range(),
            config.get_hp_circulation_config(),
            CurrentHeatDirection::None,
        ) {
            Ok(WorkingTempAction::Heat { allow_mixed: _ }) => {
                info!("Decided we should heat instead while trying circulation.");
                Ok(Intention::Finish)
            }
            Ok(WorkingTempAction::Cool { circulate: true }) => {
                debug!("Still cool/circulate, continuing to wait");
                Ok(Intention::YieldHeatUps)
            }
            Ok(WorkingTempAction::Cool { circulate: false }) => {
                info!("No longer should circulate, finishing TryCirculate");
                Ok(Intention::Finish)
            }
            Err(missing_sensor) => {
                error!(
                    "Missing {} sensor to decide whether to circulate, stopping",
                    missing_sensor
                );
                Ok(Intention::Finish)
            }
        }
    }
}
