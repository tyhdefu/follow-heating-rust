use crate::brain::modes::heating_mode::TargetTemperature;
use crate::brain::modes::working_temp::{
    find_working_temp_action, CurrentHeatDirection, WorkingTempAction,
};
use crate::brain::modes::{InfoCache, Intention, Mode};
use crate::brain::python_like::config::overrun_config::OverrunBap;
use crate::brain::python_like::config::PythonBrainConfig;
use crate::brain::python_like::control::heating_control::HeatPumpMode;
use crate::brain::BrainFailure;
use crate::expect_available;
use crate::io::IOBundle;
use crate::time_util::mytime::TimeProvider;
use crate::time_util::timeslot::ZonedSlot;
use chrono::{DateTime, SecondsFormat, Utc};
use log::{debug, error, info, warn};
use std::fmt::{Display, Formatter};
use tokio::runtime::Runtime;

#[derive(Debug, PartialEq)]
pub struct HeatUpTo {
    target: TargetTemperature,
    min_temp: Option<f32>,
    expire: HeatUpEnd,
}

impl Mode for HeatUpTo {
    fn enter(
        &mut self,
        _config: &PythonBrainConfig,
        _runtime: &Runtime,
        io_bundle: &mut IOBundle,
    ) -> Result<(), BrainFailure> {
        let heating = expect_available!(io_bundle.heating_control())?;
        heating.set_heat_pump(HeatPumpMode::HotWaterOnly, None)
    }

    fn update(
        &mut self,
        rt: &Runtime,
        config: &PythonBrainConfig,
        info_cache: &mut InfoCache,
        io_bundle: &mut IOBundle,
        time: &impl TimeProvider,
    ) -> Result<Intention, BrainFailure> {
        if self.expire.has_expired(time.get_utc_time()) {
            return Ok(Intention::finish());
        }
        let temps = rt.block_on(info_cache.get_temps(io_bundle.temperature_manager()));
        if temps.is_err() {
            error!(
                "Temperatures not available, stopping overrun {}",
                temps.unwrap_err()
            );
            return Ok(Intention::off_now());
        }
        let temps = temps.unwrap();

        let temp = match temps.get(self.get_target().get_target_sensor()) {
            Some(temp) => temp,
            None => {
                error!(
                    "Sensor {} targeted by overrun didn't have a temperature associated.",
                    self.get_target().get_target_sensor()
                );
                return Ok(Intention::off_now());
            }
        };
        info!(
            "Target: {} ({}), currently {:.2}",
            self.get_target(),
            self.get_expiry(),
            temp
        );

        if info_cache.heating_on() {
            match find_working_temp_action(
                &temps,
                &info_cache.get_working_temp_range(),
                config.get_hp_circulation_config(),
                CurrentHeatDirection::Falling,
                None,
            ) {
                Ok(WorkingTempAction::Cool { .. }) => {
                    debug!("Continuing to heat hot water as we would be circulating.");
                }
                Ok(WorkingTempAction::Heat { .. }) => {
                    info!(
                        "Call for heat during HeatUpTo, checking min {:.2?}",
                        self.min_temp,
                    );
                    if self.min_temp.is_some_and(|min| *temp < min) {
                        info!("Below minimum - Ignoring call for heat");
                    } else {
                        return Ok(Intention::finish());
                    }
                }
                Err(e) => {
                    warn!("Missing sensor {e} to determine whether we are in circulate. But we are fine how we are - staying.");
                }
            };
        }
        if *temp > self.get_target().get_target_temp() {
            info!("Reached target overrun temp.");
            return Ok(Intention::finish());
        }

        Ok(Intention::KeepState)
    }
}

#[derive(Debug, PartialEq, Clone)]
pub enum HeatUpEnd {
    Slot(ZonedSlot),
    Utc(DateTime<Utc>),
}

impl HeatUpEnd {
    pub fn has_expired(&self, now: DateTime<Utc>) -> bool {
        match self {
            HeatUpEnd::Slot(slot) => !slot.contains(&now),
            HeatUpEnd::Utc(expire_time) => now > *expire_time,
        }
    }
}

impl Display for HeatUpEnd {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            HeatUpEnd::Slot(slot) => {
                write!(f, "During {}", slot)
            }
            HeatUpEnd::Utc(time) => {
                write!(
                    f,
                    "Until {}",
                    time.to_rfc3339_opts(SecondsFormat::Millis, true)
                )
            }
        }
    }
}

impl HeatUpTo {
    pub fn from_overrun(overrun: &OverrunBap) -> Self {
        Self {
            target: TargetTemperature::new(overrun.get_sensor().clone(), overrun.get_temp()),
            expire: HeatUpEnd::Slot(overrun.get_slot().clone()),
            min_temp: *overrun.get_min_temp(),
        }
    }

    pub fn from_time(target: TargetTemperature, expire: DateTime<Utc>) -> Self {
        Self {
            target,
            expire: HeatUpEnd::Utc(expire),
            min_temp: None,
        }
    }

    pub fn get_target(&self) -> &TargetTemperature {
        &self.target
    }

    pub fn get_expiry(&self) -> &HeatUpEnd {
        &self.expire
    }
}

#[allow(clippy::zero_prefixed_literal)]
#[cfg(test)]
mod test {
    use super::*;
    use crate::brain::modes::heating_mode::TargetTemperature;
    use crate::brain::modes::working_temp::{WorkingRange, WorkingTemperatureRange};
    use crate::brain::modes::{HeatingState, InfoCache, Intention, Mode};
    use crate::brain::python_like::config::PythonBrainConfig;
    use crate::io::dummy_io_bundle::new_dummy_io;
    use crate::io::temperatures::dummy::ModifyState as TModifyState;
    use crate::io::temperatures::Sensor;
    use crate::time_util::mytime::DummyTimeProvider;
    use crate::time_util::test_utils::{date, time, utc_datetime, utc_time_slot};
    use chrono::{Duration, TimeZone, Utc};

    #[test]
    fn test_results() {
        let rt = Runtime::new().unwrap();

        let mut info_cache = InfoCache::create(
            HeatingState::OFF,
            WorkingRange::from_temp_only(WorkingTemperatureRange::from_delta(45.0, 10.0)),
        );

        let mut heat_up_to = HeatUpTo::from_overrun(&OverrunBap::new(
            utc_time_slot(10, 00, 00, 12, 00, 00),
            40.0,
            Sensor::TKBT,
        ));

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
                &PythonBrainConfig::default(),
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
                &PythonBrainConfig::default(),
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
                &PythonBrainConfig::default(),
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
        let mut mode = HeatUpTo::from_time(
            TargetTemperature::new(Sensor::TKBT, 39.0),
            utc_time + Duration::hours(1),
        );

        let rt = Runtime::new().unwrap();

        let (mut io_bundle, mut handle) = new_dummy_io();
        let time = DummyTimeProvider::new(utc_time);

        handle.send_temp(Sensor::TKBT, 30.0);
        handle.send_temp(Sensor::HXIF, 50.0);
        handle.send_temp(Sensor::HXIR, 50.0);
        handle.send_temp(Sensor::HXOR, 50.0);

        let next = mode.update(
            &rt,
            &PythonBrainConfig::default(),
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
        let mut mode = HeatUpTo::from_overrun(&OverrunBap::new_with_min(
            utc_slot.clone(),
            50.0,
            Sensor::TKBT,
            30.0,
        ));

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
        let config = PythonBrainConfig::default();

        mode.enter(&config, &rt, &mut io_bundle)?;

        let intention = mode.update(&rt, &config, &mut info_cache, &mut io_bundle, &time)?;

        assert_eq!(intention, Intention::KeepState);

        Ok(())
    }
}
