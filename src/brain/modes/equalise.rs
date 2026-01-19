use std::time::{Duration, Instant};

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
use super::working_temp::{find_working_temp_action, CurrentHeatDirection, WorkingTempAction};
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
            initial_delay: std::time::Duration::from_secs(60),
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
        heating.set_circulation_pump(true, None)
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

        let temps = rt.block_on(info_cache.get_temps(io_bundle.temperature_manager()));
        if temps.is_err() {
            error!("Failed to get temperatures, sleeping more and will keep checking.");
            return Ok(Intention::off_now());
        }

        match find_working_temp_action(
            &temps.unwrap(),
            &working_temp,
            &config,
            CurrentHeatDirection::Falling,
            None, None,
            expect_available!(io_bundle.heating_control())?.get_heat_pump_on_with_time()?.1
        ) {
            Ok((_, WorkingTempAction::Cool { circulate: true })) => {
                if self.started.elapsed() <= self.initial_delay {
                    Ok(Intention::YieldHeatUps)
                }
                else {
                    // This happened 14:16 on 4th even though above heating temp range
                    Ok(Intention::SwitchForce(HeatingMode::TryCirculate(TryCirculateMode::new(Instant::now()))))
                }
            }
            Ok((heating_mode, WorkingTempAction::Cool { circulate: false })) => {
                if self.started.elapsed() <= self.initial_delay {
                    return Ok(Intention::YieldHeatUps);
                }

                if let Some(pre_circulate @ HeatingMode::PreCirculate(_)) = heating_mode {
                    if let HeatingMode::PreCirculate(ref data) = pre_circulate {
                        if data.max_duration > Duration::from_secs(30) {
                            info!("Avoiding circulate but going into pre-circulate before deciding what to do");
                            return Ok(Intention::SwitchForce(pre_circulate))
                        }
                    }
                }

                if self.started.elapsed() > config.hp_circulation.initial_hp_sleep * 2 { // TODO: De-bodge
                    info!("TKBT too cold, would be heating the tank. Staying off.");
                    Ok(Intention::off_now())
                }
                else {
                    info!("Nothing to do - equalising for longer");
                    Ok(Intention::YieldHeatUps)
                }
            }
            Ok((_, WorkingTempAction::Heat { .. })) => {
                info!("Conditions no longer say we should cool down.");
                Ok(Intention::Finish)
            }
            Err(missing_sensor) => {
                error!("Failed to get {missing_sensor} temperature, sleeping more and will keep checking.");
                Ok(Intention::off_now())
            }
        }
    }
}
