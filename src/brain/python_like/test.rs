use crate::brain::modes::heat_up_to::HeatUpTo;
use crate::brain::modes::heating_mode::HeatingMode;
use crate::brain::modes::on::OnMode;
use crate::brain::modes::turning_on::TurningOnMode;
use crate::brain::python_like::config::PythonBrainConfig;
use crate::brain::python_like::PythonBrain;
use crate::brain::{Brain, BrainFailure};
use crate::io::dummy_io_bundle::new_dummy_io;
use crate::io::temperatures::dummy::ModifyState as TModifyState;
use crate::io::temperatures::Sensor;
use crate::io::wiser::dummy::ModifyState as WModifyState;
use crate::time_util::mytime::DummyTimeProvider;
use crate::time_util::mytime::TimeProvider;
use crate::time_util::test_utils::{date, time, utc_time_slot};
use chrono::{DateTime, Duration, TimeZone, Utc};
use log::info;
use std::time::Instant;
use tokio::runtime::Runtime;

use super::config::overrun_config::OverrunBap;

fn insignificant_time() -> DateTime<Utc> {
    Utc.from_utc_datetime(&date(2023, 12, 18).and_time(time(14, 01, 00)))
}

/// Check that the brain starts off given that the wiser heating is off and default configuration.
#[test]
fn test_stay_off() -> Result<(), BrainFailure> {
    let rt = Runtime::new().expect("Failed to create runtime.");
    let mut brain = PythonBrain::new(PythonBrainConfig::default());
    let (mut io_bundle, mut handle) = new_dummy_io();

    handle.send_wiser(WModifyState::TurnOffHeating);
    let time_provider = DummyTimeProvider::new(insignificant_time());

    assert_eq!(brain.heating_mode, None);
    brain.run(&rt, &mut io_bundle, &time_provider)?;
    assert_eq!(brain.heating_mode, Some(HeatingMode::off()));
    Ok(())
}

/// Check that the
#[test_log::test]
fn test_turning_on() -> Result<(), BrainFailure> {
    let rt = Runtime::new().expect("Failed to create runtime.");
    let mut brain = PythonBrain::new(PythonBrainConfig::default());
    let (mut io_bundle, mut handle) = new_dummy_io();

    let fixed_time = insignificant_time();

    handle.send_wiser(WModifyState::SetHeatingOffTime(
        fixed_time + Duration::seconds(10 * 60),
    ));
    handle.send_temp(Sensor::TKBT, 35.0);
    handle.send_temp(Sensor::HXIF, 35.0);
    handle.send_temp(Sensor::HXIR, 35.0);
    handle.send_temp(Sensor::HXOR, 35.0);

    let time_provider = DummyTimeProvider::new(fixed_time);

    brain.run(&rt, &mut io_bundle, &time_provider)?;
    match brain.heating_mode {
        Some(HeatingMode::TurningOn(_)) => Ok(()),
        mode => panic!(
            "Should have been in the TurningOn mode, actually in: {:?}",
            mode
        ),
    }
}

const IGNORE_WISER_CONFIG_STR: &str = r#"
[[no_heating]]
type = "Utc"
start = "13:00:00"
end = "15:00:00"

overrun_during.slots = []
"#;

/// Test that we don't go into the On mode when the wiser comes on when we are ignoring the wiser.
#[test_log::test]
fn test_ignore_wiser_while_off() -> Result<(), BrainFailure> {
    let rt = Runtime::new().expect("Failed to create runtime.");
    let config = toml::from_str(IGNORE_WISER_CONFIG_STR).expect("Failed to deserialize config");
    let mut brain = PythonBrain::new(config);
    let (mut io_bundle, mut handle) = new_dummy_io();

    let fixed_time = Utc.from_utc_datetime(&date(2023, 12, 18).and_time(time(14, 01, 00)));

    handle.send_wiser(WModifyState::SetHeatingOffTime(
        fixed_time + Duration::seconds(10 * 60),
    ));
    handle.send_temps(TModifyState::SetTemp(Sensor::TKBT, 35.0));

    let time_provider = DummyTimeProvider::new(insignificant_time());

    brain.run(&rt, &mut io_bundle, &time_provider)?;
    assert_eq!(brain.heating_mode, Some(HeatingMode::off()));

    Ok(())
}

/// Test that we come out of heating mode when we are ignoring the wiser.
#[test_log::test]
fn test_ignore_wiser_while_on() -> Result<(), BrainFailure> {
    let rt = Runtime::new().expect("Failed to create runtime.");
    let config = toml::from_str(IGNORE_WISER_CONFIG_STR).expect("Failed to deserialize config");
    let mut brain = PythonBrain::new(config);
    let (mut io_bundle, mut handle) = new_dummy_io();

    let fixed_time = Utc.from_utc_datetime(&date(2023, 12, 18).and_time(time(14, 01, 00)));

    handle.send_wiser(WModifyState::SetHeatingOffTime(
        fixed_time + Duration::seconds(10 * 60),
    ));
    handle.send_temps(TModifyState::SetTemp(Sensor::TKBT, 35.0));

    let time_provider = DummyTimeProvider::new(insignificant_time());

    let started = Instant::now() - time::Duration::minutes(10);
    brain.heating_mode = Some(HeatingMode::On(OnMode::new(true, started)));
    brain.shared_data.entered_state = started;
    brain.run(&rt, &mut io_bundle, &time_provider)?;

    assert_eq!(brain.heating_mode, Some(HeatingMode::off()));

    Ok(())
}

const IGNORE_WISER_OVERRUN_CONFIG_STR: &str = r#"
[[no_heating]]
type = "Utc"
start = "14:05:00"
end = "15:00:00"

[[overrun_during.slots]]
slot = { type = "Utc", start = "13:00:00", end = "15:00:00" }
sensor = "TKBT"
temp = 55.0
min_temp = 30.0
"#;

#[test_log::test]
fn test_ignore_wiser_into_overrun() -> Result<(), BrainFailure> {
    let rt = Runtime::new().expect("Failed to create runtime.");
    let config =
        toml::from_str(IGNORE_WISER_OVERRUN_CONFIG_STR).expect("Failed to deserialize config");
    let mut brain = PythonBrain::new(config);
    let (mut io_bundle, mut handle) = new_dummy_io();

    let fixed_time = Utc.from_utc_datetime(&date(2023, 12, 18).and_time(time(14, 01, 00)));

    handle.send_temp(Sensor::TKBT, 35.0);
    handle.send_temp(Sensor::HXIF, 35.0);
    handle.send_temp(Sensor::HXIR, 35.0);
    handle.send_temp(Sensor::HXOR, 35.0);

    let mut time_provider = DummyTimeProvider::new(insignificant_time());

    // Pretend we started turning on 10 minutes ago.
    brain.shared_data.entered_state = Instant::now() - time::Duration::minutes(10);
    brain.heating_mode = Some(HeatingMode::TurningOn(TurningOnMode::new(
        brain.shared_data.entered_state,
    )));

    // Advance 10 mins into the ignore heating.
    handle.send_wiser(WModifyState::SetHeatingOffTime(
        fixed_time + Duration::minutes(30),
    ));

    info!("-- FIRST RUN --");
    brain.run(&rt, &mut io_bundle, &time_provider)?;

    // Pretend we've been in ON long enough to not force-overrun.
    brain.shared_data.entered_state = Instant::now() - time::Duration::minutes(10);

    info!("-- SECOND RUN --");
    time_provider.advance(Duration::minutes(10));
    info!(
        "Time provider {:?}, time: {}",
        time_provider,
        time_provider.get_utc_time()
    );
    brain.run(&rt, &mut io_bundle, &time_provider)?;

    let expected_mode = HeatingMode::HeatUpTo(HeatUpTo::from_overrun(&OverrunBap::new_with_min(
        utc_time_slot(13, 00, 00, 15, 00, 00),
        55.0,
        Sensor::TKBT,
        30.0,
    )));
    assert_eq!(brain.heating_mode, Some(expected_mode));

    Ok(())
}
