use std::time::Instant;

use log::{error, info};
use tokio::runtime::Runtime;

use crate::brain::python_like::config::PythonBrainConfig;
use crate::brain::BrainFailure;
use crate::io::IOBundle;
use crate::time_util::mytime::TimeProvider;

use super::heating_mode::HeatingMode;
use super::intention::Intention;
use super::try_circulate::TryCirculateMode;
use super::working_temp::{find_working_temp_action, CurrentHeatDirection, WorkingTempAction, MixedState};
use super::{InfoCache, Mode};

#[derive(PartialEq, Debug)]
pub struct PreCirculateMode {
    started: Instant,
}

impl PreCirculateMode {
    pub fn start() -> Self {
        Self {
            started: Instant::now(),
        }
    }
}

impl Mode for PreCirculateMode {
    fn enter(
        &mut self,
        config: &PythonBrainConfig,
        _runtime: &tokio::runtime::Runtime,
        _io_bundle: &mut crate::io::IOBundle,
    ) -> Result<(), BrainFailure> {
        info!(
            "Waiting {}s in PreCirculate",
            config
                .get_hp_circulation_config()
                .get_initial_hp_sleep()
                .as_secs()
        );

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
        let working_temp = info_cache.get_working_temp_range();
        // TODO: Check working range each time.

        if &self.started.elapsed() > config.get_hp_circulation_config().get_initial_hp_sleep() {
            let temps = rt.block_on(info_cache.get_temps(io_bundle.temperature_manager()));
            if temps.is_err() {
                error!("Failed to get temperatures, sleeping more and will keep checking.");
                return Ok(Intention::off_now());
            }

            return match find_working_temp_action(
                &temps.unwrap(),
                &working_temp,
                config.get_hp_circulation_config(),
                CurrentHeatDirection::Falling,
                MixedState::NotMixed,
            ) {
                Ok(WorkingTempAction::Cool { circulate: true }) => Ok(Intention::SwitchForce(
                    HeatingMode::TryCirculate(TryCirculateMode::new(Instant::now())),
                )),
                Ok(WorkingTempAction::Cool { circulate: false }) => {
                    info!("Tank too hot to circulate, staying off.");
                    return Ok(Intention::off_now());
                }
                Ok(WorkingTempAction::Heat { .. }) => {
                    info!("Conditions no longer say we should cool down.");
                    return Ok(Intention::Finish);
                }
                Err(missing_sensor) => {
                    error!(
                        "Failed to get {} temperature, sleeping more and will keep checking.",
                        missing_sensor
                    );
                    return Ok(Intention::off_now());
                }
            };
        }

        Ok(Intention::YieldHeatUps)
    }
}
