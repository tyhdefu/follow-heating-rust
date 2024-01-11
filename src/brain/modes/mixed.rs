use log::{debug, error, info};
use tokio::runtime::Runtime;

use crate::brain::python_like::config::overrun_config::OverrunBap;
use crate::brain::python_like::config::PythonBrainConfig;
use crate::brain::python_like::control::heating_control::HeatPumpMode;
use crate::brain::BrainFailure;
use crate::expect_available;
use crate::io::IOBundle;
use crate::time_util::mytime::TimeProvider;

use super::heat_up_to::HeatUpEnd;
use super::heating_mode::TargetTemperature;
use super::intention::Intention;
use super::working_temp::{find_working_temp_action, CurrentHeatDirection, WorkingTempAction};
use super::{InfoCache, Mode};

/// Mode for running both heating and
#[derive(Debug, PartialEq)]
pub struct MixedMode {
    target_temperature: TargetTemperature,
    expire: HeatUpEnd,
}

impl MixedMode {
    pub fn new(target_temperature: TargetTemperature, expire: HeatUpEnd) -> Self {
        Self {
            target_temperature,
            expire,
        }
    }

    pub fn from_overrun(overrun: OverrunBap) -> Self {
        Self::new(
            TargetTemperature::new(overrun.get_sensor().clone(), overrun.get_temp()),
            HeatUpEnd::Slot(overrun.get_slot().clone()),
        )
    }
}

impl Mode for MixedMode {
    fn enter(
        &mut self,
        _config: &PythonBrainConfig,
        _runtime: &Runtime,
        io_bundle: &mut IOBundle,
    ) -> Result<(), BrainFailure> {
        info!(
            "Entering mixed mode, based on overrun: {} {}",
            self.target_temperature, self.expire
        );
        let heating = expect_available!(io_bundle.heating_control())?;

        if heating.try_get_heat_pump()? != HeatPumpMode::MostlyHotWater {
            debug!("Turning on HP when entering mode.");
            heating.try_set_heat_pump(HeatPumpMode::MostlyHotWater)?;
        }

        if !heating.try_get_heat_circulation_pump()? {
            debug!("Turning on CP when entering mode.");
            heating.try_set_heat_circulation_pump(true)?;
        }

        Ok(())
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
            return Ok(Intention::finish());
        }

        if self.expire.has_expired(time.get_utc_time()) {
            info!("Overrun expired");
            return Ok(Intention::finish());
        }

        let temps = match rt.block_on(info_cache.get_temps(io_bundle.temperature_manager())) {
            Ok(temps) => temps,
            Err(e) => {
                error!("Failed to retrieve temperatures: {e} - turning off.");
                return Ok(Intention::off_now());
            }
        };

        match temps.get(self.target_temperature.get_target_sensor()) {
            Some(sensor_temp) => {
                info!(
                    "{}: {:.2}, Target {} {}",
                    self.target_temperature.get_target_sensor(),
                    sensor_temp,
                    self.target_temperature,
                    self.expire
                );
                if *sensor_temp > self.target_temperature.get_target_temp() {
                    info!("Reached target temperature.");
                    return Ok(Intention::finish());
                }
            }
            None => {
                error!(
                    "Missing sensor: {}",
                    self.target_temperature.get_target_sensor()
                );
                return Ok(Intention::off_now());
            }
        };

        match find_working_temp_action(
            &temps,
            &info_cache.get_working_temp_range(),
            config.get_hp_circulation_config(),
            CurrentHeatDirection::Climbing,
        ) {
            Ok(WorkingTempAction::Heat { allow_mixed: true }) => Ok(Intention::KeepState),
            Ok(WorkingTempAction::Heat { allow_mixed: false }) => Ok(Intention::finish()),
            Ok(WorkingTempAction::Cool { circulate: _ }) => Ok(Intention::finish()),
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

    use crate::brain::modes::heat_up_to::HeatUpEnd;
    use crate::brain::modes::heating_mode::TargetTemperature;
    use crate::brain::modes::intention::Intention;
    use crate::brain::modes::working_temp::{WorkingRange, WorkingTemperatureRange};
    use crate::brain::modes::{HeatingState, InfoCache, Mode};
    use crate::brain::python_like::config::PythonBrainConfig;
    use crate::brain::BrainFailure;
    use crate::io::dummy_io_bundle::new_dummy_io;
    use crate::io::temperatures::Sensor;
    use crate::time_util::mytime::DummyTimeProvider;
    use crate::time_util::test_utils::utc_datetime;

    use super::MixedMode;

    #[test]
    fn test_finish_when_wiser_off() -> Result<(), BrainFailure> {
        let config = PythonBrainConfig::default();
        let (mut io_bundle, mut handle) = new_dummy_io();
        let range = WorkingRange::from_temp_only(WorkingTemperatureRange::from_min_max(20.0, 60.0));
        let mut info_cache = InfoCache::create(HeatingState::OFF, range.clone());
        let rt = Runtime::new().unwrap();
        let time_provider = DummyTimeProvider::new(utc_datetime(2023, 11, 14, 12, 0, 0));

        let mut mode = MixedMode::new(
            TargetTemperature::new(Sensor::TKBT, 40.0),
            HeatUpEnd::Utc(utc_datetime(2023, 11, 14, 15, 30, 20)),
        );

        handle.send_temp(Sensor::HXIF, 59.0);
        handle.send_temp(Sensor::HXIR, 59.0);
        handle.send_temp(Sensor::HXOR, 59.0);
        handle.send_temp(Sensor::TKBT, 35.5);

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
        let config = PythonBrainConfig::default();
        let (mut io_bundle, mut handle) = new_dummy_io();
        let range = WorkingRange::from_temp_only(WorkingTemperatureRange::from_min_max(20.0, 60.0));
        let mut info_cache = InfoCache::create(HeatingState::ON, range.clone());
        let rt = Runtime::new().unwrap();
        let time_provider = DummyTimeProvider::new(utc_datetime(2023, 11, 14, 12, 0, 0));

        let mut mode = MixedMode::new(
            TargetTemperature::new(Sensor::TKBT, 40.0),
            HeatUpEnd::Utc(utc_datetime(2023, 11, 14, 15, 30, 20)),
        );

        handle.send_temp(Sensor::HXIF, 59.0);
        handle.send_temp(Sensor::HXIR, 59.0);
        handle.send_temp(Sensor::HXOR, 59.0);
        handle.send_temp(Sensor::TKBT, 40.5);

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
    fn test_keep_mode_when_wiser_on_and_below_temp() -> Result<(), BrainFailure> {
        let config = PythonBrainConfig::default();
        let (mut io_bundle, mut handle) = new_dummy_io();
        let range = WorkingRange::from_temp_only(WorkingTemperatureRange::from_min_max(20.0, 60.0));
        let mut info_cache = InfoCache::create(HeatingState::ON, range.clone());
        let rt = Runtime::new().unwrap();
        let time_provider = DummyTimeProvider::new(utc_datetime(2023, 11, 14, 12, 0, 0));

        let mut mode = MixedMode::new(
            TargetTemperature::new(Sensor::TKBT, 40.0),
            HeatUpEnd::Utc(utc_datetime(2023, 11, 14, 15, 30, 20)),
        );

        handle.send_temp(Sensor::HXIF, 59.0);
        handle.send_temp(Sensor::HXIR, 59.0);
        handle.send_temp(Sensor::HXOR, 59.0);
        handle.send_temp(Sensor::TKBT, 35.5);

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
}
