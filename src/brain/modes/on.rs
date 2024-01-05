use crate::brain::modes::heat_up_to::HeatUpTo;
use crate::brain::modes::heating_mode::expect_available_fn;
use crate::brain::modes::heating_mode::{HeatingMode, SharedData};
use crate::brain::modes::intention::Intention;
use crate::brain::modes::{InfoCache, Mode};
use crate::brain::python_like::config::PythonBrainConfig;
use crate::brain::python_like::control::heating_control::HeatPumpMode;
use crate::brain::BrainFailure;
use crate::brain_fail;
use crate::expect_available;
use crate::io::temperatures::Sensor;
use crate::io::IOBundle;
use crate::time_util::mytime::TimeProvider;
use crate::CorrectiveActions;
use log::{debug, error, info, warn};
use tokio::runtime::Runtime;

use super::circulate::{should_circulate_using_forecast, CurrentHeatDirection, WorkingTempAction};

#[derive(Debug, PartialEq, Default)]
pub struct OnMode {
    circulation_pump_on: bool,
}

impl OnMode {
    pub fn new(cp_state: bool) -> Self {
        Self {
            circulation_pump_on: cp_state,
        }
    }
}

impl Mode for OnMode {
    fn enter(
        &mut self,
        _config: &PythonBrainConfig,
        _runtime: &Runtime,
        io_bundle: &mut IOBundle,
    ) -> Result<(), BrainFailure> {
        let heating = expect_available!(io_bundle.heating_control())?;

        if heating.try_get_heat_pump()? != HeatPumpMode::HeatingOnly {
            debug!("Turning on HP when entering mode.");
            heating.try_set_heat_pump(HeatPumpMode::HeatingOnly)?;
        }

        let cp = heating.try_get_heat_circulation_pump()?;
        if self.circulation_pump_on != cp {
            debug!("Setting internal circulation pump on to {}", cp);
            self.circulation_pump_on = cp;
        }
        Ok(())
    }

    fn update(
        &mut self,
        shared_data: &mut SharedData,
        rt: &Runtime,
        config: &PythonBrainConfig,
        info_cache: &mut InfoCache,
        io_bundle: &mut IOBundle,
        time: &impl TimeProvider,
    ) -> Result<Intention, BrainFailure> {
        let temps = rt.block_on(info_cache.get_temps(io_bundle.temperature_manager()));
        if let Err(err) = temps {
            error!("Failed to retrieve temperatures {}. Turning off.", err);
            return Ok(Intention::off_now());
        }
        let temps = temps.unwrap();

        if !info_cache.heating_on() {
            // TODO: 6 minute / overrun should move to Intention / tracking out of state.
            let running_for = shared_data.get_entered_state().elapsed();
            let min_runtime = config.get_min_hp_runtime();
            if running_for < *min_runtime.get_min_runtime() {
                warn!(
                    "Warning: Carrying on until the {} second mark or safety cut off: {}",
                    min_runtime.get_min_runtime().as_secs(),
                    min_runtime.get_safety_cut_off()
                );
                let remaining = *min_runtime.get_min_runtime() - running_for;
                let end = time.get_utc_time() + chrono::Duration::from_std(remaining).unwrap();
                return Ok(Intention::SwitchForce(HeatingMode::HeatUpTo(
                    HeatUpTo::from_time(min_runtime.get_safety_cut_off().clone(), end),
                )));
            }
            return Ok(Intention::finish());
        }

        match should_circulate_using_forecast(
            &temps,
            &info_cache.get_working_temp_range(),
            config.get_hp_circulation_config(),
            CurrentHeatDirection::Climbing,
        ) {
            Ok(WorkingTempAction::Heat) => {}
            Ok(_) => {
                info!("Hit top of working range - should no longer heat");
                return Ok(Intention::finish());
            }
            Err(missing_sensor) => {
                error!(
                    "Can't check whether to circulate due to missing sensor: {}",
                    missing_sensor
                );
                return Ok(Intention::off_now());
            }
        }
        if !self.circulation_pump_on {
            if let Some(temp) = temps.get(&Sensor::HPRT) {
                if *temp > config.get_temp_before_circulate() {
                    info!("Reached min circulation temp.");
                    let gpio = expect_available!(io_bundle.heating_control())?;
                    gpio.try_set_heat_circulation_pump(true)?;
                    self.circulation_pump_on = true;
                }
            }
        }
        Ok(Intention::KeepState)
    }
}
