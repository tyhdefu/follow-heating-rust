use log::*;
use tokio::runtime::Runtime;

use crate::brain::python_like::config::PythonBrainConfig;
use crate::brain::python_like::control::heating_control::HeatPumpMode;
use crate::brain::BrainFailure;
use crate::expect_available;
use crate::io::IOBundle;
use crate::time_util::mytime::TimeProvider;

use super::intention::Intention;
use super::working_temp::{find_working_temp_action, CurrentHeatDirection, WorkingTempAction, MixedState};
use super::{InfoCache, Mode, allow_dhw_mixed, AllowDhwMixed};

/// Mode for running both heating and
#[derive(Debug, PartialEq)]
pub struct MixedMode {}

impl MixedMode {
    pub fn new() -> Self {
        Self { }
    }
}

impl Mode for MixedMode {
    fn enter(
        &mut self,
        _config: &PythonBrainConfig,
        _runtime: &Runtime,
        io_bundle: &mut IOBundle,
    ) -> Result<(), BrainFailure> {
        info!("Entering mixed mode");
        let heating = expect_available!(io_bundle.heating_control())?;
        heating.set_heat_pump(HeatPumpMode::MostlyHotWater, Some("Turning on HP when entering mode."))?;
        heating.set_heat_circulation_pump(true, Some("Turning on CP when entering mode."))
    }

    fn update(
        &mut self,
        rt: &Runtime,
        config: &PythonBrainConfig,
        info_cache: &mut InfoCache,
        io_bundle: &mut IOBundle,
        time: &impl TimeProvider,
    ) -> Result<Intention, BrainFailure> {
        if !info_cache.heating_on() {
            info!("Heating is no longer on");
            return Ok(Intention::finish());
        }

        let temps = match rt.block_on(info_cache.get_temps(io_bundle.temperature_manager())) {
            Err(err) => {
                error!("Temperatures not available, stopping overrun {err}");
                return Ok(Intention::off_now());
            },
            Ok(temps) => temps,
        };

        let now = time.get_utc_time();

        let slot = config.get_overrun_during().find_matching_slot(&now, &temps,
            |temps, temp| temp < temps.extra.unwrap_or(temps.max)
        );

        let Some(slot) = slot else {
            info!("No longer matches a DHW slot");
            return Ok(Intention::finish());
        };

        match allow_dhw_mixed(&temps, slot) {
            AllowDhwMixed::Error  => return Ok(Intention::off_now()),
            AllowDhwMixed::Can |
            AllowDhwMixed::Force  => {},
            AllowDhwMixed::Cannot => return Ok(Intention::finish())
        }

        match find_working_temp_action(
            &temps,
            &info_cache.get_working_temp_range(),
            &config.hp_circulation,
            CurrentHeatDirection::Climbing,
            Some(MixedState::MixedHeating),
        ) {
            Ok(WorkingTempAction::Heat { mixed_state }) => {
                match allow_dhw_mixed(&temps, slot) {
                    AllowDhwMixed::Error  => Ok(Intention::off_now()),
                    AllowDhwMixed::Can    => {
                        if mixed_state == MixedState::MixedHeating {
                            Ok(Intention::KeepState)
                        }
                        else {
                            Ok(Intention::finish())
                        }
                    }
                    AllowDhwMixed::Force  => Ok(Intention::KeepState),
                    AllowDhwMixed::Cannot => Ok(Intention::finish()),
                }
            }
            Ok(WorkingTempAction::Cool { .. }) => Ok(Intention::finish()),
            Err(missing_sensor) => {
                error!(
                    "Could not check whether to circulate due to missing sensor: {}. Turning off",
                    missing_sensor
                );
                Ok(Intention::off_now())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use tokio::runtime::Runtime;

    use crate::brain::modes::intention::Intention;
    use crate::brain::modes::working_temp::{WorkingRange, WorkingTemperatureRange};
    use crate::brain::modes::{HeatingState, InfoCache, Mode};
    use crate::brain::python_like::config::PythonBrainConfig;
    use crate::brain::BrainFailure;
    use crate::brain::python_like::config::overrun_config::{DhwTemps, DhwBap};
    use crate::io::dummy_io_bundle::new_dummy_io;
    use crate::io::temperatures::Sensor;
    use crate::time_util::mytime::DummyTimeProvider;
    use crate::time_util::test_utils::{utc_datetime, utc_time_slot};

    use super::MixedMode;

    #[test]
    fn test_finish_when_wiser_off() -> Result<(), BrainFailure> {
        let mut config = PythonBrainConfig::default();
        let (mut io_bundle, mut handle) = new_dummy_io();
        let range = WorkingRange::from_temp_only(WorkingTemperatureRange::from_min_max(20.0, 60.0));
        let mut info_cache = InfoCache::create(HeatingState::OFF, range.clone());
        let rt = Runtime::new().unwrap();
        let time_provider = DummyTimeProvider::new(utc_datetime(2023, 11, 14, 12, 0, 0));

        config._add_dhw_slot(DhwBap::_new(
            utc_time_slot(11,0,0, 15,30,20),
            Sensor::TKBT, 0.0, 40.0,
        ));

        handle.send_temp(Sensor::HXIF, 59.0);
        handle.send_temp(Sensor::HXIR, 59.0);
        handle.send_temp(Sensor::HXOR, 59.0);
        handle.send_temp(Sensor::TKBT, 35.5);
        handle.send_temp(Sensor::HPRT, 50.0);

        let mut mode = MixedMode::new();

        mode.enter(&config, &rt, &mut io_bundle)?;

        let intention = mode.update(
            &rt,
            &config,
            &mut info_cache,
            &mut io_bundle,
            &time_provider,
        )?;

        assert_eq!(intention, Intention::Finish);

        Ok(())
    }

    #[test]
    fn test_finish_when_temp_reached() -> Result<(), BrainFailure> {
        let mut config = PythonBrainConfig::default();
        let (mut io_bundle, mut handle) = new_dummy_io();
        let range = WorkingRange::from_temp_only(WorkingTemperatureRange::from_min_max(20.0, 60.0));
        let mut info_cache = InfoCache::create(HeatingState::ON, range.clone());
        let rt = Runtime::new().unwrap();
        let time_provider = DummyTimeProvider::new(utc_datetime(2023, 11, 14, 12, 0, 0));

        config._add_dhw_slot(DhwBap::_new(
            utc_time_slot(11,0,0, 15,30,20),
            Sensor::TKBT, 0.0, 40.0,
        ));

        handle.send_temp(Sensor::HXIF, 59.0);
        handle.send_temp(Sensor::HXIR, 59.0);
        handle.send_temp(Sensor::HXOR, 59.0);
        handle.send_temp(Sensor::TKBT, 40.5);
        handle.send_temp(Sensor::HPRT, 50.0);

        let mut mode = MixedMode::new();

        mode.enter(&config, &rt, &mut io_bundle)?;

        let intention = mode.update(
            &rt,
            &config,
            &mut info_cache,
            &mut io_bundle,
            &time_provider,
        )?;

        assert_eq!(intention, Intention::Finish);

        Ok(())
    }

    #[test]
    fn test_continue_when_wiser_on_and_within_temp() -> Result<(), BrainFailure> {
        let mut config = PythonBrainConfig::default();
        let (mut io_bundle, mut handle) = new_dummy_io();
        let range = WorkingRange::from_temp_only(WorkingTemperatureRange::from_min_max(20.0, 60.0));
        let mut info_cache = InfoCache::create(HeatingState::ON, range.clone());
        let rt = Runtime::new().unwrap();
        let time_provider = DummyTimeProvider::new(utc_datetime(2023, 11, 14, 12, 0, 0));

        config._add_dhw_slot(DhwBap::_new(
            utc_time_slot(11,0,0, 15,30,20),
            Sensor::TKBT, 0.0, 40.0,
        ));

        handle.send_temp(Sensor::HXIF, 59.0);
        handle.send_temp(Sensor::HXIR, 59.0);
        handle.send_temp(Sensor::HXOR, 59.0);
        handle.send_temp(Sensor::TKBT, 35.5);

        handle.send_temp(Sensor::TKFL, 20.0);
        handle.send_temp(Sensor::HPFL, 30.0);
        handle.send_temp(Sensor::HPRT, 50.0);

        let mut mode = MixedMode::new();

        mode.enter(&config, &rt, &mut io_bundle)?;

        let intention = mode.update(
            &rt,
            &config,
            &mut info_cache,
            &mut io_bundle,
            &time_provider,
        )?;

        assert_eq!(intention, Intention::KeepState);

        Ok(())
    }

    #[test]
    fn test_finish_when_wiser_on_and_below_temp() -> Result<(), BrainFailure> {
        let mut config = PythonBrainConfig::default();
        let (mut io_bundle, mut handle) = new_dummy_io();
        let range = WorkingRange::from_temp_only(WorkingTemperatureRange::from_min_max(20.0, 60.0));
        let mut info_cache = InfoCache::create(HeatingState::ON, range.clone());
        let rt = Runtime::new().unwrap();
        let time_provider = DummyTimeProvider::new(utc_datetime(2023, 11, 14, 12, 0, 0));

        config._add_dhw_slot(DhwBap::_new(
            utc_time_slot(11,0,0, 15,30,20),
            Sensor::TKBT, 36.0, 40.0,
        ));

        handle.send_temp(Sensor::HXIF, 59.0);
        handle.send_temp(Sensor::HXIR, 59.0);
        handle.send_temp(Sensor::HXOR, 59.0);
        handle.send_temp(Sensor::TKBT, 35.5);

        handle.send_temp(Sensor::TKFL, 20.0);
        handle.send_temp(Sensor::HPFL, 30.0);
        handle.send_temp(Sensor::HPRT, 50.0);

        let mut mode = MixedMode::new();

        mode.enter(&config, &rt, &mut io_bundle)?;

        let intention = mode.update(
            &rt,
            &config,
            &mut info_cache,
            &mut io_bundle,
            &time_provider,
        )?;

        assert_eq!(intention, Intention::Finish);

        Ok(())
    }
}
