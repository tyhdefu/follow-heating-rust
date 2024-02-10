use std::time::Instant;

use log::{error, info};
use tokio::runtime::Runtime;

use crate::brain::python_like::config::PythonBrainConfig;
use crate::brain::BrainFailure;
use crate::brain::python_like::control::heating_control::HeatPumpMode;
use crate::expect_available;
use crate::io::IOBundle;
use crate::time_util::mytime::TimeProvider;

use super::heating_mode::HeatingMode;
use super::intention::Intention;
use super::try_circulate::TryCirculateMode;
use super::working_temp::{find_working_temp_action, CurrentHeatDirection, WorkingTempAction, MixedState};
use super::{InfoCache, Mode};

#[derive(PartialEq, Debug)]
pub struct EqualiseMode {
    started: Instant,
    initial_delay: std::time::Duration,
}

impl EqualiseMode {
    pub fn start() -> Self {
        Self {
            started: Instant::now(),
            initial_delay: std::time::Duration::from_secs(40),
        }
    }
}

impl Mode for EqualiseMode {
    fn enter(
        &mut self,
        _config: &PythonBrainConfig,
        _runtime: &tokio::runtime::Runtime,
        io_bundle: &mut crate::io::IOBundle,
    ) -> Result<(), BrainFailure> {
        info!("Waiting {}s in EqualiseMode", self.initial_delay.as_secs());

        let heating = expect_available!(io_bundle.heating_control())?;
        heating.set_heat_pump(HeatPumpMode::Off, None)?;
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
            return Ok(Intention::Finish);
        }
        let working_temp = info_cache.get_working_temp_range();
        // TODO: Check working range each time.

        if self.started.elapsed() <= self.initial_delay {
            return Ok(Intention::YieldHeatUps);
        }

        let temps = rt.block_on(info_cache.get_temps(io_bundle.temperature_manager()));
        if temps.is_err() {
            error!("Failed to get temperatures, sleeping more and will keep checking.");
            return Ok(Intention::off_now());
        }

        match find_working_temp_action(
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
                if &self.started.elapsed() > config.get_hp_circulation_config().get_initial_hp_sleep() {
                    info!("TKBT too cold, would be heating the tank. Staying off.");
                    Ok(Intention::off_now())
                }
                else {
                    info!("Nothing to do - equalising for longer");
                    Ok(Intention::YieldHeatUps)
                }
            }
            Ok(WorkingTempAction::Heat { .. }) => {
                info!("Conditions no longer say we should cool down.");
                Ok(Intention::Finish)
            }
            Err(missing_sensor) => {
                error!(
                    "Failed to get {} temperature, sleeping more and will keep checking.",
                    missing_sensor
                );
                Ok(Intention::off_now())
            }
        }       
    }
}
