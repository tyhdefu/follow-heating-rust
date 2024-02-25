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
use crate::time_util::timeslot::ZonedSlot;
use chrono::{DateTime, SecondsFormat, Utc};
use log::{debug, error, info, warn};
use std::fmt::{Display, Formatter};
use std::time::Duration;
use tokio::runtime::Runtime;

#[derive(Debug, PartialEq)]
pub struct DhwOnlyMode {
}

impl Mode for DhwOnlyMode {
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
        let short_duration = hp_duration < Duration::from_secs(60 * 10);

        let slot = config.get_overrun_during().find_matching_slot(&now, &temps,
            |temps, temp| temp < temps.max || (short_duration && temp < temps.extra.unwrap_or(temps.max))
        );

        let Some(slot) = slot else {
            info!("No longer matches a DHW slot");
            return Ok(Intention::finish());
        };

        let temp = match temps.get(&slot.temps.sensor) {
            Some(temp) => temp,
            None => {
                error!("Sensor {} targeted by overrun didn't have a temperature associated.", slot.temps.sensor);
                return Ok(Intention::off_now());
            }
        };
        info!("Target: {:.1}-{:.1}/{:.1?} until {}, currently {:.2}", slot.temps.min, slot.temps.max, short_duration.then_some(slot.temps.extra), slot.slot, temp); //TODO: Remove

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
                    info!("Call for heat during HeatUpTo, checking min {:.2?}", slot.temps.min);
                    if *temp < slot.temps.min {
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

        if let Some(bypass) = &slot.bypass {
            let diff = temps.get(&Sensor::HPFL).unwrap_or(&0.0) - temps.get(&Sensor::HPRT).unwrap_or(&0.0);
            if heating_control.try_get_heat_pump()? == HeatPumpMode::HotWaterOnlyWithBypass {
                if diff <= bypass.end_hp_drop {
                    info!("Bypass no longer required as HPFL-HPRT={diff:.1}");
                    heating_control.set_heat_pump(HeatPumpMode::HotWaterOnly, None)?;
                }
            }
            else {
                if diff >= bypass.start_hp_drop {
                    info!("Bypass required as HPFL-HPRT={diff:.1}");
                    heating_control.set_heat_pump(HeatPumpMode::HotWaterOnlyWithBypass, None)?;
                }
            }
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

impl DhwOnlyMode {
    pub fn new() -> Self {
        Self {}
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
    use crate::brain::python_like::config::overrun_config::{DhwBap, DhwTemps};

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

        let mut heat_up_to = DhwOnlyMode::new();

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

        let mut mode = DhwOnlyMode::new();

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

        let mut mode = DhwOnlyMode::new();
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
