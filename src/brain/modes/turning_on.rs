use std::time::Instant;

use crate::brain::modes::heating_mode::expect_available_fn;
use crate::brain_fail;
use crate::CorrectiveActions;
use log::debug;
use log::info;
use tokio::runtime::Runtime;

use crate::{
    brain::{python_like::config::PythonBrainConfig, BrainFailure},
    expect_available,
    io::IOBundle,
    time_util::mytime::TimeProvider,
};

use super::{intention::Intention, InfoCache, Mode};

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

        if !heating.try_get_heat_pump()? {
            debug!("Turning on HP when entering mode.");
            heating.try_set_heat_pump(true)?;
        }
        if !heating.try_get_heat_circulation_pump()? {
            debug!("Turning on CP when entering mode.");
            heating.try_set_heat_circulation_pump(true)?;
        }
        Ok(())
    }

    fn update(
        &mut self,
        _shared_data: &mut super::heating_mode::SharedData,
        _rt: &Runtime,
        config: &PythonBrainConfig,
        info_cache: &mut InfoCache,
        _io_bundle: &mut IOBundle,
        _time: &impl TimeProvider,
    ) -> Result<Intention, BrainFailure> {
        if !info_cache.heating_on() {
            info!("Wiser turned off before waiting time period ended");
            // TODO: Should it potentially go into overrun from this? - if not, need to switch off
            // immediately.
            return Ok(Intention::finish());
        }

        /*let temps = match rt.block_on(info_cache.get_temps(io_bundle.temperature_manager())) {
            Ok(temps) => temps,
            Err(e) => {
                error!(
                    "Failed to retrieve temperatures '{}'. Cancelling TurningOn",
                    e
                );
                return Ok(Intention::off_now());
            }
        };

        if let Some(temp) = temps.get(&Sensor::HPRT) {
            let heating_control = expect_available!(io_bundle.heating_control())?;
            if *temp > config.get_temp_before_circulate()
                && !heating_control.try_get_heat_circulation_pump()?
            {
                info!("Reached min circulation temperature while turning on, turning on circulation pump.");
                heating_control.try_set_heat_circulation_pump(true)?
            }
        }*/

        if &self.started.elapsed() > config.get_hp_enable_time() {
            return Ok(Intention::finish());
        }
        Ok(Intention::KeepState)
    }
}
