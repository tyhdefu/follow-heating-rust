use std::time::{Duration, Instant};

use log::*;
use tokio::runtime::Runtime;

use crate::brain::modes::working_temp::{CurrentHeatDirection, WorkingTempAction, find_working_temp_action};
use crate::brain::python_like::config::PythonBrainConfig;
use crate::brain::BrainFailure;
use crate::expect_available;
use crate::io::IOBundle;
use crate::time_util::mytime::TimeProvider;

use super::equalise::EqualiseMode;
use super::heating_mode::HeatingMode;
use super::intention::Intention;
use super::{InfoCache, Mode};

#[derive(PartialEq, Debug)]
pub struct PreCirculateMode {
    max_duration: Duration,
    started:      Instant,
}

impl PreCirculateMode {
    pub fn new(max_duration: Duration) -> Self {
        Self {
            max_duration,
            started: Instant::now(),
        }
    }
}

impl Mode for PreCirculateMode {
    fn enter(
        &mut self,
        _config: &PythonBrainConfig,
        _runtime: &tokio::runtime::Runtime,
        _io_bundle: &mut crate::io::IOBundle,
    ) -> Result<(), BrainFailure> {
        info!("Waiting up to {}s in PreCirculate", self.max_duration.as_secs());
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
        // TODO: Check working range each time. (I think this refers to heating hot water?)

        let temps = rt.block_on(info_cache.get_temps(io_bundle.temperature_manager()));
        if temps.is_err() {
            error!("Failed to get temperatures, sleeping more and will keep checking.");
            return Ok(Intention::off_now());
        }

        match find_working_temp_action(
            &temps.unwrap(),
            &working_temp,
            &config.hp_circulation,
            CurrentHeatDirection::Falling,
            None, None,
            expect_available!(io_bundle.heating_control())?.as_hp().get_heat_pump_on_with_time()?.1
        ) {
            Ok((_, WorkingTempAction::Heat { .. })) => {
                info!("Don't even need to circulate to see temperature apparently below threshold");
                Ok(Intention::Finish)
            }
            Err(missing_sensor) => {
                error!("Failed to get {missing_sensor} temperature, sleeping more and will keep checking.");
                Ok(Intention::off_now())
            }
            _ => {
                if self.started.elapsed() > self.max_duration {
                    Ok(Intention::SwitchForce(HeatingMode::Equalise(EqualiseMode::start())))
                }
                else {
                    Ok(Intention::YieldHeatUps)
                }
            }
        }
    }
}
