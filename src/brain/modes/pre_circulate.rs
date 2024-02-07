use std::time::Instant;

use log::*;
use tokio::runtime::Runtime;

use crate::brain::python_like::config::PythonBrainConfig;
use crate::brain::BrainFailure;
use crate::io::IOBundle;
use crate::time_util::mytime::TimeProvider;

use super::equalise::EqualiseMode;
use super::heating_mode::HeatingMode;
use super::intention::Intention;
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
        _rt: &Runtime,
        config: &PythonBrainConfig,
        info_cache: &mut InfoCache,
        _io_bundle: &mut IOBundle,
        _time: &impl TimeProvider,
    ) -> Result<Intention, BrainFailure> {
        if !info_cache.heating_on() {
            return Ok(Intention::Finish);
        }

        // TODO: Check working range each time.

        if &self.started.elapsed() > config.get_hp_circulation_config().get_initial_hp_sleep() {
            Ok(Intention::SwitchForce(
                HeatingMode::Equalise(EqualiseMode::start()),
            ))
        }
        else {
            Ok(Intention::YieldHeatUps)
        }
    }
}
