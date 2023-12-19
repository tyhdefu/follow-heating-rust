use log::warn;
use tokio::runtime::Runtime;
use crate::brain::BrainFailure;
use crate::brain::modes::heating_mode::SharedData;
use crate::brain::modes::{InfoCache, Mode};
use crate::brain::modes::intention::Intention;
use crate::brain::python_like::config::PythonBrainConfig;
use crate::expect_available;
use crate::brain_fail;
use crate::CorrectiveActions;
use crate::brain::modes::heating_mode::expect_available_fn;
use crate::io::IOBundle;
use crate::time_util::mytime::TimeProvider;

/// Mode that represents where everything is off
/// The program can be safely terminated when in this mode.
#[derive(Default, PartialEq, Debug)]
pub struct OffMode {}

impl Mode for OffMode {
    fn enter(&mut self, _config: &PythonBrainConfig, _runtime: &Runtime, io_bundle: &mut IOBundle) -> Result<(), BrainFailure> {
        let heating = expect_available!(io_bundle.heating_control())?;
        if heating.try_get_heat_pump()? {
            warn!("Entering Off Mode - turning off Heat Pump");
            heating.try_set_heat_pump(false)?;
        }
        if heating.try_get_heat_circulation_pump()? {
            warn!("Entering Off Mode - turning off Heat Circulation Pump");
            heating.try_set_heat_pump(false)?;
        }

        Ok(())
    }

    fn update(&mut self, _shared_data: &mut SharedData, _rt: &Runtime, _config: &PythonBrainConfig, _info_cache: &mut InfoCache, _io_bundle: &mut IOBundle, _time: &impl TimeProvider) -> Result<Intention, BrainFailure> {
        // Do nothing, return logic to intention repeatedly.
        return Ok(Intention::finish());
    }
}