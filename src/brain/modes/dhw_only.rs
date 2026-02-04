use crate::brain::modes::heating_mode::get_dhw_only_or_nothing;
use crate::brain::modes::working_temp::{
    find_working_temp_action, CurrentHeatDirection, WorkingTempAction,
};
use crate::brain::modes::{InfoCache, Intention, Mode};
use crate::brain::python_like::config::PythonBrainConfig;
use crate::brain::python_like::control::heating_control::HeatPumpMode;
use crate::brain::BrainFailure;
use crate::expect_available;
use crate::io::IOBundle;
use crate::io::temperatures::Sensor;
use crate::time_util::mytime::TimeProvider;
use log::{debug, error, info, warn};
use tokio::runtime::Runtime;

use super::working_temp::MixedState;
use super::heating_mode::HeatingMode;
use super::mixed::MixedMode;

/// Why we entered this mode. If we came from off mode then the heating definitely
/// didn't overshoot and can go into MixedMode as soon as there's demand, otherwise
/// no.
#[derive(Debug, PartialEq)]
pub enum DidHeatingOvershoot {
    Yes, No, NotSure
}

#[derive(Debug, PartialEq)]
pub struct DhwOnlyMode {
    did_heating_overshoot: DidHeatingOvershoot,
}

impl DhwOnlyMode {
    pub fn new(did_heating_overshoot: DidHeatingOvershoot) -> Self {
        Self { did_heating_overshoot }
    }
}

impl Mode for DhwOnlyMode {
    fn enter(
        &mut self,
        _config: &PythonBrainConfig,
        _runtime: &Runtime,
        io_bundle: &mut IOBundle,
    ) -> Result<(), BrainFailure> {
        info!("Entering DhwOnlyMode with: did_heating_overshoot={:?}", self.did_heating_overshot);
        let heating = expect_available!(io_bundle.heating_control())?;
        heating.set_heat_pump(HeatPumpMode::HotWaterOnly, None)?;
        heating.set_circulation_pump(false, None)
    }

    fn update(
        &mut self,
        rt: &Runtime,
        config: &PythonBrainConfig,
        info_cache: &mut InfoCache,
        io_bundle: &mut IOBundle,
        time: &impl TimeProvider,
    ) -> Result<Intention, BrainFailure> {
        let temps = match rt.block_on(info_cache.get_temps(io_bundle.temperature_manager())) {
            Err(err) => {
                error!("Temperatures not available, stopping overrun {err}");
                return Ok(Intention::off_now());
            },
            Ok(temps) => temps,
        };

        let now = time.get_utc_time();

        let heating_control = expect_available!(io_bundle.heating_control())?;
        let (_hp_on, hp_duration) = heating_control.get_heat_pump_on_with_time()?;

        let Some(slot) = get_dhw_only_or_nothing(config, now, hp_duration, &temps, true) else {
            info!("No longer matches a DHW slot");
            return Ok(Intention::finish());
        };

        if info_cache.heating_on() {
            match find_working_temp_action(
                &temps,
                &info_cache.get_working_temp_range(),
                &config,
                if self.did_heating_overshoot == DidHeatingOvershoot::Yes {
                    CurrentHeatDirection::Climbing
                } else {
                    CurrentHeatDirection::Falling
                },
                None,
                Some(slot),
                hp_duration,
            ) {
                Ok((Some(heating_mode @ HeatingMode::Mixed(_)), _)) => {
                    return Ok(Intention::SwitchForce(heating_mode));
                }
                Ok((Some(heating_mode @ HeatingMode::DhwOnly(_)), _)) => {
                    debug!("Continuing to heat hot water as find_working_temp_action says so");
                }
                Ok((_, WorkingTempAction::Cool { .. })) => {
                    debug!("Continuing to heat hot water as we would be circulating.");
                }
                Ok((_, WorkingTempAction::Heat { mixed_state })) => {
                    if mixed_state == MixedState::MixedHeating {
                        warn!("Legacy code path DhwOnly -> Mixed");
                        return Ok(Intention::SwitchForce(HeatingMode::Mixed(MixedMode::new())))
                    }
                    warn!("Unexpected code path DhwOnly -> Finish");
                    return Ok(Intention::finish());
                }
                Err(e) => {
                    warn!("Missing sensor {e} to determine whether we are in circulate. But we are fine how we are - staying.");
                }
            };
        }

        if let Some(bypass) = &slot.bypass {
            let diff = temps.get(&Sensor::HPFL).unwrap_or(&0.0) - temps.get(&Sensor::HPRT).unwrap_or(&0.0);
            match heating_control.try_get_heat_pump()? {
                HeatPumpMode::MostlyHotWater => {
                    if diff <= bypass.stop_hp_drop {
                        info!("Bypass no longer required as HPFL-HPRT={diff:.1}");
                        heating_control.set_heat_pump(HeatPumpMode::HotWaterOnly, None)?;
                    }
                },
                HeatPumpMode::HotWaterOnly => {
                    if diff >= bypass.start_hp_drop {
                        info!("Bypass required as HPFL-HPRT={diff:.1}");
                        heating_control.set_heat_pump(HeatPumpMode::MostlyHotWater, None)?;
                    }
                },
                mode => {
                    error!("Unexpected mode {mode:?}");
                },
            }
        }

        Ok(Intention::KeepState)
    }
}

#[allow(clippy::zero_prefixed_literal)]
#[cfg(test)]
mod test {
    use super::*;
    use crate::brain::modes::working_temp::{WorkingRange, WorkingTemperatureRange};
    use crate::brain::modes::{HeatingState, InfoCache, Intention, Mode};
    use crate::brain::python_like::config::PythonBrainConfig;
    use crate::io::dummy_io_bundle::new_dummy_io;
    use crate::io::temperatures::dummy::ModifyState as TModifyState;
    use crate::io::temperatures::Sensor;
    use crate::time_util::mytime::DummyTimeProvider;
    use crate::time_util::test_utils::{date, time, utc_datetime, utc_time_slot};
    use chrono::{TimeZone, Utc};
    use crate::brain::python_like::config::overrun_config::DhwBap;

    #[test]
    fn test_results() {
        let rt = Runtime::new().unwrap();

        let mut info_cache = InfoCache::create(
            HeatingState::OFF,
            WorkingRange::from_temp_only(WorkingTemperatureRange::from_delta(45.0, 10.0)),
        );

        let mut config = PythonBrainConfig::default();
        config._add_dhw_slot(DhwBap::_new(
            utc_time_slot(10,00,00, 12,00,00),
            Sensor::TKBT, 10.0, 40.0
        ));

        let mut heat_up_to = DhwOnlyMode::new(DidHeatingOvershoot::NotSure);

        let (mut io_bundle, mut io_handle) = new_dummy_io();

        let date = date(2022, 02, 13);

        let in_range_time = time(11, 00, 00);
        let out_of_range_time = time(13, 00, 00);

        {
            // Keep state, still heating up.
            io_handle.send_temps(TModifyState::SetTemp(Sensor::TKBT, 35.0));
            let time_provider =
                DummyTimeProvider::new(Utc.from_utc_datetime(&date.and_time(in_range_time)));

            let result = heat_up_to.update(
                &rt,
                &config,
                &mut info_cache,
                &mut io_bundle,
                &time_provider,
            );

            let intention = result.expect("Should not have error");
            assert!(
                matches!(intention, Intention::KeepState),
                "Intention should have been KeepState but was: {:?}",
                intention
            );
            info_cache.reset_cache();
        }

        {
            ///// Check it ends when TKBT is too high. /////
            io_handle.send_temp(Sensor::TKBT, 50.0);

            let time_provider =
                DummyTimeProvider::new(Utc.from_utc_datetime(&date.and_time(in_range_time)));

            let result = heat_up_to.update(
                &rt,
                &config,
                &mut info_cache,
                &mut io_bundle,
                &time_provider,
            );

            let intention = result.expect("Should have not been any error");
            assert!(
                matches!(intention, Intention::Finish),
                "Should have finished due to high temp, actually: {:?}",
                intention
            );
            info_cache.reset_cache();
        }

        {
            ///// Check it ends when time is out of range. /////
            io_handle.send_temps(TModifyState::SetTemp(Sensor::TKBT, 35.0));

            let time_provider =
                DummyTimeProvider::new(Utc.from_utc_datetime(&date.and_time(out_of_range_time)));

            let result = heat_up_to.update(
                &rt,
                &config,
                &mut info_cache,
                &mut io_bundle,
                &time_provider,
            );

            let intention = result.expect("Should have not been any error");
            assert!(
                matches!(intention, Intention::Finish),
                "Should have been finished due to out of time range, actually: {:?}",
                intention
            );
            info_cache.reset_cache();
        }
    }

    #[test]
    fn test_stay_heatupto_when_circulating() -> Result<(), BrainFailure> {
        let working_range = WorkingTemperatureRange::from_min_max(40.0, 50.0);
        let mut info_cache = InfoCache::create(
            HeatingState::ON,
            WorkingRange::from_temp_only(working_range.clone()),
        );

        let utc_time = utc_datetime(2023, 06, 12, 10, 00, 00);

        let mut config = PythonBrainConfig::default();

        config._add_dhw_slot(DhwBap::_new(
            utc_time_slot(09,00,00, 11,00,00),
            Sensor::TKBT, 0.0, 39.0
        ));

        let mut mode = DhwOnlyMode::new(DidHeatingOvershoot::NotSure);

        let rt = Runtime::new().unwrap();

        let (mut io_bundle, mut handle) = new_dummy_io();
        let time = DummyTimeProvider::new(utc_time);

        handle.send_temp(Sensor::TKBT, 30.0);
        handle.send_temp(Sensor::HXIF, 50.0);
        handle.send_temp(Sensor::HXIR, 50.0);
        handle.send_temp(Sensor::HXOR, 50.0);

        let next = mode.update(
            &rt,
            &config,
            &mut info_cache,
            &mut io_bundle,
            &time,
        )?;

        assert!(
            matches!(next, Intention::KeepState),
            "Should be KeepState. Was: {:?}",
            next
        );

        Ok(())
    }

    #[test]
    fn test_stay_heatupto_when_below_min() -> Result<(), BrainFailure> {
        let working_range = WorkingTemperatureRange::from_min_max(40.0, 50.0);

        let utc_slot = utc_time_slot(12, 0, 0, 13, 0, 0);

        let mut config = PythonBrainConfig::default();

        config._add_dhw_slot(DhwBap::_new(
            utc_slot.clone(),
            Sensor::TKBT, 30.0, 50.0
        ));

        let mut mode = DhwOnlyMode::new(DidHeatingOvershoot::NotSure);
        let rt = Runtime::new().unwrap();
        let (mut io_bundle, mut handle) = new_dummy_io();
        let time = DummyTimeProvider::in_slot(&utc_slot);

        handle.send_temp(Sensor::TKBT, 20.0);
        handle.send_temp(Sensor::HXIF, 39.5);
        handle.send_temp(Sensor::HXIR, 39.5);
        handle.send_temp(Sensor::HXOR, 39.5);

        let mut info_cache = InfoCache::create(
            HeatingState::ON,
            WorkingRange::from_temp_only(working_range),
        );

        mode.enter(&config, &rt, &mut io_bundle)?;

        let intention = mode.update(&rt, &config, &mut info_cache, &mut io_bundle, &time)?;

        assert_eq!(intention, Intention::KeepState);

        Ok(())
    }
}
