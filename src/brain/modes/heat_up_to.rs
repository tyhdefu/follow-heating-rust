use crate::brain::modes::heating_mode::expect_available_fn;
use crate::brain::modes::heating_mode::{SharedData, TargetTemperature};
use crate::brain::modes::{InfoCache, Intention, Mode};
use crate::brain::python_like::config::PythonBrainConfig;
use crate::brain::BrainFailure;
use crate::brain_fail;
use crate::expect_available;
use crate::io::IOBundle;
use crate::time_util::mytime::TimeProvider;
use crate::time_util::timeslot::ZonedSlot;
use crate::CorrectiveActions;
use chrono::{DateTime, SecondsFormat, Utc};
use log::{error, info};
use std::fmt::{Display, Formatter};
use tokio::runtime::Runtime;

#[derive(Debug, PartialEq)]
pub struct HeatUpTo {
    target: TargetTemperature,
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
        if !heating.try_get_heat_pump()? {
            heating.try_set_heat_pump(true)?;
        }
        Ok(())
    }

    fn update(
        &mut self,
        _shared_data: &mut SharedData,
        rt: &Runtime,
        _config: &PythonBrainConfig,
        info_cache: &mut InfoCache,
        io_bundle: &mut IOBundle,
        time: &impl TimeProvider,
    ) -> Result<Intention, BrainFailure> {
        if info_cache.heating_on() {
            return Ok(Intention::finish());
        }

        if self.has_expired(time.get_utc_time()) {
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
        info!("Target {} ({})", self.get_target(), self.get_expiry());
        if let Some(temp) = temps.get(self.get_target().get_target_sensor()) {
            info!("{}: {:.2}", self.get_target().get_target_sensor(), temp);
            if *temp > self.get_target().get_target_temp() {
                info!("Reached target overrun temp.");
                return Ok(Intention::finish());
            }
        } else {
            error!(
                "Sensor {} targeted by overrun didn't have a temperature associated.",
                self.get_target().get_target_sensor()
            );
            return Ok(Intention::off_now());
        }
        Ok(Intention::KeepState)
    }
}

#[derive(Debug, PartialEq, Clone)]
pub enum HeatUpEnd {
    Slot(ZonedSlot),
    Utc(DateTime<Utc>),
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
    pub fn from_slot(target: TargetTemperature, expire: ZonedSlot) -> Self {
        Self {
            target,
            expire: HeatUpEnd::Slot(expire),
        }
    }

    pub fn from_time(target: TargetTemperature, expire: DateTime<Utc>) -> Self {
        Self {
            target,
            expire: HeatUpEnd::Utc(expire),
        }
    }

    pub fn has_expired(&self, now: DateTime<Utc>) -> bool {
        match &self.expire {
            HeatUpEnd::Slot(slot) => !slot.contains(&now),
            HeatUpEnd::Utc(expire_time) => now > *expire_time,
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
    use crate::brain::modes::heating_mode::{SharedData, TargetTemperature};
    use crate::brain::modes::{HeatingState, InfoCache, Intention, Mode};
    use crate::brain::python_like::config::PythonBrainConfig;
    use crate::brain::python_like::working_temp::{WorkingRange, WorkingTemperatureRange};
    use crate::brain::python_like::FallbackWorkingRange;
    use crate::io::dummy_io_bundle::new_dummy_io;
    use crate::io::temperatures::dummy::ModifyState;
    use crate::io::temperatures::Sensor;
    use crate::time_util::mytime::DummyTimeProvider;
    use crate::time_util::test_utils::{date, time, utc_time_slot};
    use chrono::{TimeZone, Utc};
    use tokio::runtime::Builder;

    #[test]
    fn test_results() {
        let mut shared_data = SharedData::new(FallbackWorkingRange::new(
            WorkingTemperatureRange::from_delta(50.0, 10.0),
        ));
        let rt = Builder::new_multi_thread()
            .worker_threads(1)
            .enable_time()
            .enable_io()
            .build()
            .expect("Expected to be able to make runtime");

        let mut info_cache = InfoCache::create(
            HeatingState::OFF,
            WorkingRange::from_temp_only(WorkingTemperatureRange::from_delta(45.0, 10.0)),
        );

        let mut heat_up_to = HeatUpTo::from_slot(
            TargetTemperature::new(Sensor::TKBT, 40.0),
            utc_time_slot(10, 00, 00, 12, 00, 00),
        );

        let (mut io_bundle, mut io_handle) = new_dummy_io();

        let date = date(2022, 02, 13);

        let in_range_time = time(11, 00, 00);
        let out_of_range_time = time(13, 00, 00);

        {
            // Keep state, still heating up.
            io_handle.send_temps(ModifyState::SetTemp(Sensor::TKBT, 35.0));
            let time_provider =
                DummyTimeProvider::new(Utc.from_utc_datetime(&date.and_time(in_range_time)));

            let result = heat_up_to.update(
                &mut shared_data,
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
            io_handle.send_temps(ModifyState::SetTemp(Sensor::TKBT, 50.0));

            let time_provider =
                DummyTimeProvider::new(Utc.from_utc_datetime(&date.and_time(in_range_time)));

            let result = heat_up_to.update(
                &mut shared_data,
                &rt,
                &PythonBrainConfig::default(),
                &mut info_cache,
                &mut io_bundle,
                &time_provider,
            );

            let intention = result.expect("Should have not been any error");
            assert!(
                matches!(intention, Intention::FinishMode),
                "Should have finished due to high temp, actually: {:?}",
                intention
            );
            info_cache.reset_cache();
        }

        {
            ///// Check it ends when time is out of range. /////
            io_handle.send_temps(ModifyState::SetTemp(Sensor::TKBT, 35.0));

            let time_provider =
                DummyTimeProvider::new(Utc.from_utc_datetime(&date.and_time(out_of_range_time)));

            let result = heat_up_to.update(
                &mut shared_data,
                &rt,
                &PythonBrainConfig::default(),
                &mut info_cache,
                &mut io_bundle,
                &time_provider,
            );

            let intention = result.expect("Should have not been any error");
            assert!(
                matches!(intention, Intention::FinishMode),
                "Should have been finished due to out of time range, actually: {:?}",
                intention
            );
            info_cache.reset_cache();
        }
    }
}
