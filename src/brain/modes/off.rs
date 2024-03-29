use crate::brain::modes::intention::Intention;
use crate::brain::modes::{InfoCache, Mode};
use crate::brain::python_like::config::PythonBrainConfig;
use crate::brain::python_like::control::heating_control::HeatPumpMode;
use crate::brain::BrainFailure;
use crate::expect_available;
use crate::io::IOBundle;
use crate::time_util::mytime::TimeProvider;
use tokio::runtime::Runtime;

/// Mode that represents where everything is off
/// The program can be safely terminated when in this mode.
#[derive(Default, PartialEq, Debug)]
pub struct OffMode {}

impl Mode for OffMode {
    fn enter(
        &mut self,
        _config: &PythonBrainConfig,
        _runtime: &Runtime,
        io_bundle: &mut IOBundle,
    ) -> Result<(), BrainFailure> {
        let heating = expect_available!(io_bundle.heating_control())?;
        heating.set_heat_pump(HeatPumpMode::Off, Some("Entering Off Mode - turning off Heat Pump"))?;
        heating.set_heat_circulation_pump(false, Some("Entering Off Mode - turning off Heat Circulation Pump"))
    }

    fn update(
        &mut self,
        _rt: &Runtime,
        _config: &PythonBrainConfig,
        _info_cache: &mut InfoCache,
        _io_bundle: &mut IOBundle,
        _time: &impl TimeProvider,
    ) -> Result<Intention, BrainFailure> {
        // Do nothing, return logic to intention repeatedly.
        Ok(Intention::finish())
    }
}
