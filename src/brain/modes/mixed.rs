use log::*;
use tokio::runtime::Runtime;

use crate::brain::python_like::config::overrun_config::{DhwBap, DhwTemps};
use crate::brain::python_like::config::PythonBrainConfig;
use crate::brain::python_like::control::heating_control::HeatPumpMode;
use crate::brain::BrainFailure;
use crate::expect_available;
use crate::io::IOBundle;
use crate::time_util::mytime::TimeProvider;

use super::heat_up_to::HeatUpEnd;
use super::intention::Intention;
use super::working_temp::{find_working_temp_action, CurrentHeatDirection, WorkingTempAction, MixedState};
use super::{InfoCache, Mode};

/// Mode for running both heating and
#[derive(Debug, PartialEq)]
pub struct MixedMode {
    temps: DhwTemps,
    expire: HeatUpEnd,
}

impl MixedMode {
    pub fn new(temps: DhwTemps, expire: HeatUpEnd) -> Self {
        Self { temps, expire }
    }

    pub fn from_overrun(dhw: &DhwBap) -> Self {
        Self::new(
            dhw.temps.clone(),
            HeatUpEnd::Slot(dhw.slot.clone()),
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
            self.temps.max, self.expire
        );
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

        match temps.get(&self.temps.sensor) {
            Some(sensor_temp) => {
                info!(
                    "{}: {:.2}, Target {} {}",
                    self.temps.sensor,
                    sensor_temp,
                    self.temps.max,
                    self.expire
                );
                if *sensor_temp > self.temps.max {
                    //let (hp_on, hp_on_time) = expect_available!(io_bundle.heating_control())?.get_heat_pump_on_with_time();
                    if Some(*sensor_temp) > self.temps.extra {
                        info!("Reached target temperature.");
                        return Ok(Intention::finish());
                    }
                    else {
                        info!("Enjoying MixedMode so extending until {}", self.temps.extra.unwrap_or(0.0));
                    }
                }
            }
            None => {
                error!("Missing sensor: {}", self.temps.sensor);
                return Ok(Intention::off_now());
            }
        };

        match find_working_temp_action(
            &temps,
            &info_cache.get_working_temp_range(),
            config.get_hp_circulation_config(),
            CurrentHeatDirection::Climbing,
            Some(MixedState::MixedHeating),
        ) {
            Ok(WorkingTempAction::Heat { mixed_state: MixedState::MixedHeating }) => Ok(Intention::YieldHeatUps),
            Ok(WorkingTempAction::Heat { mixed_state: _ }) => Ok(Intention::finish()),
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

    use crate::brain::modes::heat_up_to::HeatUpEnd;
    use crate::brain::modes::intention::Intention;
    use crate::brain::modes::working_temp::{WorkingRange, WorkingTemperatureRange};
    use crate::brain::modes::{HeatingState, InfoCache, Mode};
    use crate::brain::python_like::config::PythonBrainConfig;
    use crate::brain::BrainFailure;
    use crate::brain::python_like::config::overrun_config::DhwTemps;
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
            DhwTemps { sensor: Sensor::TKBT, min: 0.0, max: 40.0, extra: None },
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
            DhwTemps { sensor: Sensor::TKBT, min: 0.0, max: 40.0, extra: None },
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
    fn test_yield_heat_ups_when_wiser_on_and_below_temp() -> Result<(), BrainFailure> {
        let config = PythonBrainConfig::default();
        let (mut io_bundle, mut handle) = new_dummy_io();
        let range = WorkingRange::from_temp_only(WorkingTemperatureRange::from_min_max(20.0, 60.0));
        let mut info_cache = InfoCache::create(HeatingState::ON, range.clone());
        let rt = Runtime::new().unwrap();
        let time_provider = DummyTimeProvider::new(utc_datetime(2023, 11, 14, 12, 0, 0));

        let mut mode = MixedMode::new(
            DhwTemps { sensor: Sensor::TKBT, min: 0.0, max: 40.0, extra: None },
            HeatUpEnd::Utc(utc_datetime(2023, 11, 14, 15, 30, 20)),
        );

        handle.send_temp(Sensor::HXIF, 59.0);
        handle.send_temp(Sensor::HXIR, 59.0);
        handle.send_temp(Sensor::HXOR, 59.0);
        handle.send_temp(Sensor::TKBT, 35.5);

        handle.send_temp(Sensor::TKFL, 20.0);
        handle.send_temp(Sensor::HPFL, 30.0);

        mode.enter(&config, &rt, &mut io_bundle)?;

        let intention = mode.update(
            &rt,
            &config,
            &mut info_cache,
            &mut io_bundle,
            &time_provider,
        )?;

        assert_eq!(intention, Intention::YieldHeatUps);

        Ok(())
    }
}
