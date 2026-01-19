use crate::brain::modes::dhw_only::DhwOnlyMode;
use crate::brain::modes::working_temp::{Room, WorkingRange, WorkingTemperatureRange};
use crate::io::dummy_io_bundle::new_dummy_io;
use crate::io::temperatures::dummy::ModifyState;
use crate::python_like::control::heating_control::HeatingControl;
use crate::time_util::mytime::RealTimeProvider;
use crate::time_util::test_utils::{date, time};
use crate::{wiser, GPIOState};
use chrono::{TimeZone, Utc};
use std::thread::sleep;
use std::time::Duration;
use tokio::runtime::Builder;

use super::*;

struct CleanupHandle<'a> {
    io_bundle: &'a mut IOBundle,
    heating_mode: HeatingMode,
}

impl<'a> CleanupHandle<'a> {
    pub fn new(io_bundle: &'a mut IOBundle, heating_mode: HeatingMode) -> Self {
        Self {
            io_bundle,
            heating_mode,
        }
    }

    pub fn get_io_bundle(&mut self) -> &mut IOBundle {
        self.io_bundle
    }

    pub fn update(
        &mut self,
        shared_data: &mut SharedData,
        runtime: &Runtime,
        config: &PythonBrainConfig,
        info_cache: &mut InfoCache,
    ) -> Result<Option<HeatingMode>, BrainFailure> {
        self.heating_mode.update(
            shared_data,
            runtime,
            config,
            self.io_bundle,
            info_cache,
            &RealTimeProvider::default(),
        )
    }
}

impl Drop for CleanupHandle<'_> {
    fn drop(&mut self) {
        self.io_bundle
            .heating_control()
            .rob_or_get_now()
            .expect("Should have been able to rob gpio access.");

        // Reset pins.
        let gpio = expect_present(self.io_bundle.heating_control());
        print_state(gpio);
        gpio.set_heat_pump(HeatPumpMode::Off, Some("Drop handler"))
            .expect("Should be able to turn off HP");
        gpio.try_set_circulation_pump(false)
            .expect("Should be able to turn off CP");
    }
}

fn expect_present(gpio: &mut Dispatchable<Box<dyn HeatingControl>>) -> &mut dyn HeatingControl {
    if let Dispatchable::Available(gpio) = gpio {
        return gpio.deref_mut().borrow_mut();
    }
    panic!("GPIO not available.");
}

fn print_state(gpio: &dyn HeatingControl) {
    let state = gpio.try_get_heat_pump().unwrap();
    println!("HP GPIO state {:?}", state);

    let state = gpio.get_circulation_pump().unwrap();
    println!("CP GPIO state {:?}", state);
}

#[test_log::test]
//#[test]
pub fn test_transitions() -> Result<(), BrainFailure> {
    let (mut io_bundle, mut io_handle) = new_dummy_io();

    let rt = Builder::new_multi_thread()
        .worker_threads(1)
        .enable_time()
        .enable_io()
        .build()
        .expect("Expected to be able to make runtime");

    let config = PythonBrainConfig::default();

    let mut shared_data = SharedData::new(FallbackWorkingRange::new(
        config.default_working_range.clone(),
    ));

    fn test_transition_fn<'a>(
        mut from: HeatingMode,
        mut to: HeatingMode,
        config: &PythonBrainConfig,
        rt: &Runtime,
        io_bundle: &'a mut IOBundle,
    ) -> Result<CleanupHandle<'a>, BrainFailure> {
        println!("-- Testing {:?} -> {:?} --", from, to);

        println!("- Pre");
        print_state(expect_present(io_bundle.heating_control()));

        from.enter(config, rt, io_bundle)?;

        println!("- Init");
        print_state(expect_present(io_bundle.heating_control()));

        let entry_preferences = to.get_entry_preferences().clone();
        let transition_msg = format!("transition {:?} -> {:?}", from, to);

        from.exit_to(&to, io_bundle)?;

        {
            let gpio = expect_present(io_bundle.heating_control());

            println!("- Exited");
            print_state(gpio);

            let hp_mode = gpio.try_get_heat_pump().unwrap();
            println!("HP State {:?}", hp_mode);

            println!("HP mode: {:?}", hp_mode);
            if !entry_preferences.allow_heat_pump_on {
                assert!(
                    hp_mode.is_hp_off(),
                    "HP should be off between {}",
                    transition_msg
                );
            } else if hp_mode.is_hp_off() {
                println!("Leaving on HP correctly.");
            }

            let state = gpio.get_circulation_pump().unwrap();
            println!("CP State: {:?}", state);

            println!("CP on: {}", state.0);
            if !entry_preferences.allow_circulation_pump_on {
                assert!(!state.0, "CP should be off between {}", transition_msg);
            } else if state.0 {
                println!("Leaving on CP correctly.");
            }
        }

        to.enter(config, rt, io_bundle)?;

        Ok(CleanupHandle::new(io_bundle, to))
    }

    {
        io_handle.send_wiser(wiser::dummy::ModifyState::SetHeatingOffTime(
            Utc::now() + chrono::Duration::seconds(1000),
        ));
        let heating_on = true;

        let mut handle = test_transition_fn(
            HeatingMode::off(),
            HeatingMode::TurningOn(TurningOnMode::new(Instant::now())),
            &config,
            &rt,
            &mut io_bundle,
        )?;
        {
            let gpio = expect_present(handle.get_io_bundle().heating_control());
            assert_eq!(
                gpio.try_get_heat_pump()?,
                HeatPumpMode::HeatingOnly,
                "HP should be on"
            );
            assert_eq!(
                gpio.get_circulation_pump()?.0,
                true,
                "CP should be off"
            );
        }

        println!("Updating state.");
        io_handle.send_temps(ModifyState::SetTemp(Sensor::HXIF, 35.0));
        io_handle.send_temps(ModifyState::SetTemp(Sensor::HXIR, 35.0));
        io_handle.send_temps(ModifyState::SetTemp(Sensor::TKBT, 35.0));
        io_handle.send_temps(ModifyState::SetTemp(Sensor::HXOR, 25.0));
        io_handle.send_temps(ModifyState::SetTemp(Sensor::HPRT, 50.0));
        let mut cache = InfoCache::create(
            HeatingState::new(heating_on),
            WorkingRange::from_temp_only(WorkingTemperatureRange::from_min_max(30.0, 50.0)),
        );
        handle
            .update(&mut shared_data, &rt, &config, &mut cache)
            .unwrap();
        {
            let gpio = expect_present(handle.get_io_bundle().heating_control());
            assert_eq!(
                gpio.try_get_heat_pump()?,
                HeatPumpMode::HeatingOnly,
                "HP should be on"
            );
            assert_eq!(
                gpio.get_circulation_pump()?.0,
                true,
                "CP should be on"
            );
        }
    }

    let mut test_transition_between = |from: HeatingMode, to: HeatingMode| {
        test_transition_fn(from, to, &config, &rt, &mut io_bundle).map(|_| ())
    };

    test_transition_between(HeatingMode::On(OnMode::default()), HeatingMode::off())?;
    test_transition_between(
        HeatingMode::PreCirculate(PreCirculateMode::new()),
        HeatingMode::off(),
    )?;
    test_transition_between(
        HeatingMode::off(),
        HeatingMode::Circulate(CirculateMode::default()),
    )?;
    test_transition_between(
        HeatingMode::off(),
        HeatingMode::TurningOn(TurningOnMode::new(Instant::now())),
    )?;
    test_transition_between(
        HeatingMode::On(OnMode::default()),
        HeatingMode::Circulate(CirculateMode::default()),
    )?;
    test_transition_between(
        HeatingMode::Circulate(CirculateMode::default()),
        HeatingMode::off(),
    )?;
    test_transition_between(
        HeatingMode::Circulate(CirculateMode::default()),
        HeatingMode::TurningOn(TurningOnMode::new(Instant::now())),
    )?;
    test_transition_between(
        HeatingMode::Circulate(CirculateMode::default()),
        HeatingMode::On(OnMode::default()),
    )?;
    

    test_transition_between(
        HeatingMode::DhwOnly(DhwOnlyMode::new()),
            //DhwTemps { sensor: Sensor::TKBT, min: 0.0, max: 47.0, extra: None }, NOW
        HeatingMode::off(),
    )?;

    Ok(())
}

#[test]
pub fn test_circulation_exit() -> Result<(), BrainFailure> {
    let (mut io_bundle, mut handle) = new_dummy_io();

    let rt = Builder::new_multi_thread()
        .worker_threads(1)
        .enable_time()
        .enable_io()
        .build()
        .expect("Expected to be able to make runtime");

    let config = PythonBrainConfig::default();

    let mut shared_data = SharedData::new(FallbackWorkingRange::new(
        config.default_working_range.clone(),
    ));

    let time_provider = RealTimeProvider::default();
    {
        let mut mode = HeatingMode::Circulate(CirculateMode::default());
        handle.send_wiser(wiser::dummy::ModifyState::TurnOffHeating);
        let mut info_cache = InfoCache::create(
            HeatingState::OFF,
            WorkingRange::from_temp_only(WorkingTemperatureRange::from_min_max(30.0, 50.0)),
        );
        let next = mode.update(
            &mut shared_data,
            &rt,
            &config,
            &mut io_bundle,
            &mut info_cache,
            &time_provider,
        )?;
        assert!(
            matches!(next, Some(HeatingMode::Off(_))),
            "Should be stopping, was: {:?}",
            next
        );
        sleep(Duration::from_secs(3));
        let next_mode = mode.update(
            &mut shared_data,
            &rt,
            &config,
            &mut io_bundle,
            &mut info_cache,
            &time_provider,
        )?;
        println!("Next mode: {:?}", next_mode);
        assert!(matches!(next_mode, Some(HeatingMode::Off(_))));
    }
    Ok(())
}

#[test]
pub fn test() {
    let state = GPIOState::High;
    assert!(matches!(state, GPIOState::High), "Expected High == High");
    assert!(!matches!(state, GPIOState::Low), "Expected High != Low")
}

#[test]
fn test_overrun_scenarios() {
    let config_str = std::fs::read_to_string("test/python_brain/test_overrun_scenarios.toml")
        .expect("Failed to read config file.");
    println!("Config str: {}", config_str);
    println!();
    let config: PythonBrainConfig =
        toml::from_str(&config_str).expect("Failed to deserialize config");
    let overrun_config = config.get_overrun_during();
    println!("Overrun config: {:?}", overrun_config);
    println!();

    let mut temps = HashMap::new();
    temps.insert(Sensor::TKTP, 52.0);
    temps.insert(Sensor::TKBT, 20.0);

    let datetime = Utc::from_utc_datetime(&Utc, &date(2022, 05, 09).and_time(time(03, 10, 00)));

    let mode = get_heatup_while_off(&datetime, overrun_config, &temps);
    println!("Mode: {:?}", mode);
    assert!(mode.is_some());
    if let HeatingMode::DhwOnly(heat_up_to) = mode.unwrap() {
        // Nothing else to check
    } else {
        panic!("Should have been heat up to mode.")
    }

    temps.insert(Sensor::TKTP, 52.0);
    temps.insert(Sensor::TKBT, 46.0);
    let mode = get_heatup_while_off(&datetime, overrun_config, &temps);
    println!("Mode: {:?}", mode);
    assert!(mode.is_none());
}

#[test]
fn test_intention_change() {
    let (mut io_bundle, mut io_handle) = new_dummy_io();

    let mut info_cache = InfoCache::create(
        HeatingState::OFF,
        WorkingRange::from_temp_only(WorkingTemperatureRange::from_min_max(30.0, 50.0)),
    );

    let rt = Builder::new_multi_thread()
        .worker_threads(1)
        .enable_time()
        .enable_io()
        .build()
        .expect("Expected to be able to make runtime");

    let default_config = PythonBrainConfig::default();

    let time = Utc.from_utc_datetime(&date(2022, 03, 12).and_time(time(12, 30, 00)));

    // Heating off and no overrun.
    let off_result = handle_intention(
        Intention::Finish,
        &mut info_cache,
        &mut io_bundle,
        &default_config,
        &rt,
        &time,
    )
    .expect("Should succeed");
    assert!(matches!(off_result, Some(HeatingMode::Off(_))));

    // Overrun normal
    {
        let mut info_cache = InfoCache::create(
            HeatingState::OFF,
            WorkingRange::from_temp_only(WorkingTemperatureRange::from_min_max(30.0, 50.0)),
        );
        expect_present(io_bundle.heating_control())
            .try_set_heat_pump(HeatPumpMode::HotWaterOnly)
            .expect("Should be able to turn on.");

        let overrun_config_str = r#"
[[overrun_during.slots]]
slot = { type = "Utc", start="11:00:00", end="13:00:05" }
temps = { sensor = "TKBT", min = 0.0, max = 44.0 }
"#;
        println!("{}", overrun_config_str);
        io_handle.send_temps(ModifyState::SetTemp(Sensor::TKBT, 40.0)); // Should overrun up to 44.0 at TKBT

        let overrun_config: PythonBrainConfig =
            toml::from_str(overrun_config_str).expect("Invalid config string");

        let overrun_result = handle_intention(
            Intention::Finish,
            &mut info_cache,
            &mut io_bundle,
            &overrun_config,
            &rt,
            &time,
        )
        .expect("Should succeed");
        assert!(
            matches!(overrun_result, Some(HeatingMode::DhwOnly(_))),
            "Should have overran from finishing mode, got: {:?}",
            overrun_result
        );
        io_handle.send_temps(ModifyState::SetTemps(HashMap::new()));

        expect_present(io_bundle.heating_control())
            .set_heat_pump(HeatPumpMode::Off, Some("Test"))
            .expect("Should be able to turn off.");
    }

    // Turn off when both off
    {
        let mut info_cache = InfoCache::create(
            HeatingState::OFF,
            WorkingRange::from_temp_only(WorkingTemperatureRange::from_min_max(30.0, 50.0)),
        );

        let overrun_config_str = r#"
[[overrun_during.slots]]
slot = { type = "Utc", start="11:00:00", end="13:00:05" }
temps = { sensor = "TKBT", min = 0.0, max = 44.0 }
"#;
        println!("{}", overrun_config_str);
        io_handle.send_temps(ModifyState::SetTemp(Sensor::TKBT, 44.0));

        let overrun_config: PythonBrainConfig =
            toml::from_str(overrun_config_str).expect("Invalid config string");

        let overrun_result = handle_intention(
            Intention::Finish,
            &mut info_cache,
            &mut io_bundle,
            &overrun_config,
            &rt,
            &time,
        )
        .expect("Should succeed");
        assert!(
            matches!(overrun_result, Some(HeatingMode::Off(_))),
            "Should have turned off after finishing mode and heat pump and wiser of {:?}",
            overrun_result
        );
        io_handle.send_temps(ModifyState::SetTemps(HashMap::new()));
    }

    // Go to TurningOn, (deferring decision) if above working temp range
    {
        let mut info_cache = InfoCache::create(
            HeatingState::ON,
            WorkingRange::from_wiser(
                WorkingTemperatureRange::from_min_max(40.0, 50.0),
                Room::of("My Room".into(), 0.3, 0.3),
            ),
        );

        io_handle.send_temps(ModifyState::SetTemp(Sensor::TKBT, 10.0));
        io_handle.send_temps(ModifyState::SetTemp(Sensor::HXIF, 10.0));
        io_handle.send_temps(ModifyState::SetTemp(Sensor::HXIR, 10.0));
        io_handle.send_temps(ModifyState::SetTemp(Sensor::HXOR, 10.0));
        io_handle.send_temps(ModifyState::SetTemp(Sensor::HPRT, 50.0));

        let turning_on = handle_intention(
            Intention::Finish,
            &mut info_cache,
            &mut io_bundle,
            &default_config,
            &rt,
            &time,
        )
        .expect("Should succeed");
        assert!(
            matches!(turning_on, Some(HeatingMode::TurningOn(_))),
            "Expected TurningOn but got {:?}",
            turning_on
        );
    }
}

#[test]
fn test_intention_basic() {
    let time = Utc.from_utc_datetime(&date(2022, 03, 12).and_time(time(12, 30, 00)));

    let (mut io_bundle, _io_handle) = new_dummy_io();

    let mut info_cache = InfoCache::create(
        HeatingState::ON,
        WorkingRange::from_temp_only(WorkingTemperatureRange::from_min_max(30.0, 50.0)),
    );

    let rt = Builder::new_multi_thread()
        .worker_threads(1)
        .enable_time()
        .enable_io()
        .build()
        .expect("Expected to be able to make runtime");

    let switch_off_force = handle_intention(
        Intention::SwitchForce(HeatingMode::off()),
        &mut info_cache,
        &mut io_bundle,
        &Default::default(),
        &rt,
        &time,
    )
    .unwrap();
    assert!(
        matches!(switch_off_force, Some(HeatingMode::Off(_))),
        "Forcing switch off should lead to off."
    );

    let keep_state = handle_intention(
        Intention::KeepState,
        &mut info_cache,
        &mut io_bundle,
        &Default::default(),
        &rt,
        &time,
    )
    .unwrap();
    assert!(keep_state.is_none(), "Keep state should lead to None");
}
