use std::net::Ipv4Addr;
use std::thread::sleep;
use chrono::{NaiveDate, NaiveDateTime, NaiveTime, Utc, TimeZone};
use tokio::runtime::Builder;
use crate::{DummyAllOutputs, DummyIO, GPIOState, temperatures, wiser, WiserConfig};
use crate::brain::python_like::modes::circulate::StoppingStatus;
use crate::python_like::control::heating_control::{HeatingControl};
use crate::python_like::heating_mode;
use crate::python_like::heatupto::HeatUpTo;
use crate::temperatures::dummy::ModifyState;
use crate::time::test_utils::{date, time};

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

    pub fn get_heating_mode(&mut self) -> &mut HeatingMode {
        &mut self.heating_mode
    }

    pub fn update(&mut self, shared_data: &mut SharedData, runtime: &Runtime, config: &PythonBrainConfig) -> Result<Option<HeatingMode>, BrainFailure> {
        self.heating_mode.update(shared_data, runtime, config, self.io_bundle)
    }
}

impl Drop for CleanupHandle<'_> {
    fn drop(&mut self) {
        self.io_bundle.heating_control().rob_or_get_now().expect("Should have been able to rob gpio access.");

        // Reset pins.
        let gpio = expect_present(self.io_bundle.heating_control());
        print_state(gpio);
        gpio.try_set_heat_pump(false).expect("Should be able to turn off HP");
        gpio.try_set_heat_circulation_pump(false).expect("Should be able to turn off CP");
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

    let state = gpio.try_get_heat_circulation_pump().unwrap();
    println!("CP GPIO state {:?}", state);
}

#[test]
pub fn test_transitions() -> Result<(), BrainFailure> {
    let heating_control = DummyAllOutputs::default();
    let misc_control = DummyAllOutputs::default();
    let (wiser, wiser_handle) = wiser::dummy::Dummy::create(&WiserConfig::new(Ipv4Addr::new(0, 0, 0, 0).into(), String::new()));
    let (temp_manager, temp_handle) = temperatures::dummy::Dummy::create(&());

    let mut io_bundle = IOBundle::new(temp_manager, heating_control, misc_control, wiser);

    let rt = Builder::new_multi_thread()
        .worker_threads(1)
        .enable_time()
        .enable_io()
        .build()
        .expect("Expected to be able to make runtime");

    let config = PythonBrainConfig::default();

    let mut shared_data = SharedData::new(FallbackWorkingRange::new(config.get_default_working_range().clone()));

    fn test_transition_fn<'a>(mut from: HeatingMode, mut to: HeatingMode, config: &PythonBrainConfig, rt: &Runtime, io_bundle: &'a mut IOBundle) -> Result<CleanupHandle<'a>, BrainFailure> {
        println!("-- Testing {:?} -> {:?} --", from, to);

        println!("- Pre");
        print_state(expect_present(io_bundle.heating_control()));

        from.enter(&config, &rt, io_bundle)?;

        println!("- Init");
        print_state(expect_present(io_bundle.heating_control()));

        let entry_preferences = to.get_entry_preferences().clone();
        let transition_msg = format!("transition {:?} -> {:?}", from, to);

        from.exit_to(&to, io_bundle)?;

        {
            let gpio = expect_present(io_bundle.heating_control());

            println!("- Exited");
            print_state(gpio);

            let on = gpio.try_get_heat_pump().unwrap();
            println!("HP State {:?}", on);

            println!("HP on: {}", on);
            if !entry_preferences.allow_heat_pump_on {
                assert!(!on, "HP should be off between {}", transition_msg);
            } else if on {
                println!("Leaving on HP correctly.");
            }

            let state = gpio.try_get_heat_circulation_pump().unwrap();
            println!("CP State: {:?}", state);

            println!("CP on: {}", on);
            if !entry_preferences.allow_circulation_pump_on {
                assert!(!on, "CP should be off between {}", transition_msg);
            } else if on {
                println!("Leaving on CP correctly.");
            }
        }

        to.enter(&config, &rt, io_bundle)?;

        Ok(CleanupHandle::new(io_bundle, to))
    }

    {
        wiser_handle.send(wiser::dummy::ModifyState::SetHeatingOffTime(Utc::now() + chrono::Duration::seconds(1000))).unwrap();
        let mut handle = test_transition_fn(HeatingMode::Off, HeatingMode::On(HeatingOnStatus::default()),
                                            &config, &rt, &mut io_bundle)?;
        {
            let gpio = expect_present(handle.get_io_bundle().heating_control());
            assert_eq!(gpio.try_get_heat_pump()?, true, "HP should be on");
            assert_eq!(gpio.try_get_heat_circulation_pump()?, false, "CP should be off");
        }

        println!("Updating state.");
        temp_handle.send(ModifyState::SetTemp(Sensor::HPRT, 35.0)).unwrap();
        temp_handle.send(ModifyState::SetTemp(Sensor::TKBT, 35.0)).unwrap();
        handle.update(&mut shared_data, &rt, &config).unwrap();
        {
            let gpio = expect_present(handle.get_io_bundle().heating_control());
            assert_eq!(gpio.try_get_heat_pump()?, true, "HP should be on");
            assert_eq!(gpio.try_get_heat_circulation_pump()?, true, "CP should be on");
        }
    }

    let mut test_transition_between = |mut from: HeatingMode, mut to: HeatingMode| {
        test_transition_fn(from, to, &config, &rt, &mut io_bundle).map(|_| ())
    };

    test_transition_between(HeatingMode::On(HeatingOnStatus::default()), HeatingMode::Off)?;
    test_transition_between(HeatingMode::PreCirculate(Instant::now()), HeatingMode::Off)?;
    test_transition_between(HeatingMode::Off, HeatingMode::Circulate(CirculateStatus::Uninitialised))?;
    test_transition_between(HeatingMode::Off, HeatingMode::TurningOn(Instant::now()))?;

    test_transition_between(HeatingMode::On(HeatingOnStatus::default()), HeatingMode::Circulate(CirculateStatus::Uninitialised))?;
    test_transition_between(HeatingMode::Circulate(CirculateStatus::Stopping(StoppingStatus::stopped())), HeatingMode::Off)?;
    test_transition_between(HeatingMode::Circulate(CirculateStatus::Stopping(StoppingStatus::stopped())), HeatingMode::TurningOn(Instant::now()))?;
    test_transition_between(HeatingMode::Circulate(CirculateStatus::Stopping(StoppingStatus::stopped())), HeatingMode::On(HeatingOnStatus::default()))?;
    test_transition_between(HeatingMode::HeatUpTo(HeatUpTo::from_time(TargetTemperature::new(Sensor::TKBT, 47.0), Utc::now())), HeatingMode::Off)?;

    Ok(())
}

#[test]
pub fn test_circulation_exit() -> Result<(), BrainFailure> {
    let heating_control = DummyAllOutputs::default();
    let misc_control = DummyAllOutputs::default();
    let (wiser, wiser_handle) = wiser::dummy::Dummy::create(&WiserConfig::new(Ipv4Addr::new(0, 0, 0, 0).into(), String::new()));
    let (temp_manager, _temp_handle) = temperatures::dummy::Dummy::create(&());

    let mut io_bundle = IOBundle::new(temp_manager, heating_control, misc_control, wiser);

    let rt = Builder::new_multi_thread()
        .worker_threads(1)
        .enable_time()
        .enable_io()
        .build()
        .expect("Expected to be able to make runtime");

    let config = PythonBrainConfig::default();

    let mut shared_data = SharedData::new(FallbackWorkingRange::new(config.get_default_working_range().clone()));

    let task = cycling::start_task(&rt, io_bundle.dispatch_heating_control().unwrap(), config.get_hp_circulation_config().clone());
    {
        let mut mode = HeatingMode::Circulate(CirculateStatus::Active(task));
        wiser_handle.send(wiser::dummy::ModifyState::TurnOffHeating).unwrap();
        mode.update(&mut shared_data, &rt, &config, &mut io_bundle)?;
        assert!(matches!(mode, HeatingMode::Circulate(CirculateStatus::Stopping(_))), "Should be stopping, was: {:?}", mode);
        sleep(Duration::from_secs(3));
        let next_mode = mode.update(&mut shared_data, &rt, &config, &mut io_bundle)?;
        println!("Next mode: {:?}", next_mode);
        assert!(matches!(next_mode, Some(HeatingMode::Off)));
    }
    Ok(())
}

#[test]
pub fn test() {
    let state = GPIOState::HIGH;
    assert!(matches!(state, GPIOState::HIGH), "Expected High == High");
    assert!(!matches!(state, GPIOState::LOW), "Expected High != Low")
}

#[test]
fn test_overrun_scenarios() {
    let config_str = std::fs::read_to_string("test/python_brain/test_overrun_scenarios.toml").expect("Failed to read config file.");
    println!("Config str: {}", config_str);
    println!();
    let config: PythonBrainConfig = toml::from_str(&config_str).expect("Failed to deserialize config");
    let overrun_config = config.get_overrun_during();
    println!("Overrun config: {:?}", overrun_config);
    println!();

    let mut temps = HashMap::new();
    temps.insert(Sensor::TKTP, 52.0);
    temps.insert(Sensor::TKBT, 20.0);

    let datetime = Utc::from_utc_datetime(&Utc, &NaiveDateTime::new(NaiveDate::from_ymd(2022, 05, 09), NaiveTime::from_hms(03, 10, 00)));

    let mode = heating_mode::get_heatup_while_off(&datetime, &overrun_config, &temps);
    println!("Mode: {:?}", mode);
    assert!(mode.is_some());
    if let HeatingMode::HeatUpTo(heat_up_to) = mode.unwrap() {
        assert_eq!(heat_up_to.get_target().sensor, Sensor::TKBT);
        assert_eq!(heat_up_to.get_target().temp, 46.0) // Fine to have this lower of the two as it will increase anyway if needed.
    } else {
        panic!("Should have been heat up to mode.")
    }

    temps.insert(Sensor::TKTP, 52.0);
    temps.insert(Sensor::TKBT, 46.0);
    let mode = heating_mode::get_heatup_while_off(&datetime, &overrun_config, &temps);
    println!("Mode: {:?}", mode);
    assert!(mode.is_none());
}

#[test]
fn test_intention_change() {
    let heating_control = DummyAllOutputs::default();
    let misc_control = DummyAllOutputs::default();
    let (wiser, _wiser_handle) = wiser::dummy::Dummy::create(&WiserConfig::new(Ipv4Addr::new(0, 0, 0, 0).into(), String::new()));
    let (temp_manager, temp_handle) = temperatures::dummy::Dummy::create(&());

    let mut io_bundle = IOBundle::new(temp_manager, heating_control, misc_control, wiser);

    let mut info_cache = InfoCache::create(false, WorkingRange::from_temp_only(WorkingTemperatureRange::from_min_max(30.0, 50.0)));

    let rt = Builder::new_multi_thread()
        .worker_threads(1)
        .enable_time()
        .enable_io()
        .build()
        .expect("Expected to be able to make runtime");

    let default_config = PythonBrainConfig::default();

    let time = Utc.from_utc_datetime(&date(2022, 03, 12).and_time(time(12, 30, 00)));

    // Heating off and no overrun.
    let off_result = handle_intention(Intention::Change(ChangeState::FinishMode), &mut info_cache, &mut io_bundle, &default_config, &rt, &time).expect("Should succeed");
    assert!(matches!(off_result, Some(HeatingMode::Off)));

    // Overrun normal
    {
        let mut info_cache = InfoCache::create(false, WorkingRange::from_temp_only(WorkingTemperatureRange::from_min_max(30.0, 50.0)));
        expect_present(io_bundle.heating_control())
            .try_set_heat_pump(true).expect("Should be able to turn on.");

        let overrun_config_str = r#"
[[overrun_during.slots]]
slot = { type = "Utc", start="11:00:00", end="13:00:05" }
sensor = "TKBT"
temp = 44.0
"#;
        println!("{}", overrun_config_str);
        temp_handle.send(ModifyState::SetTemp(Sensor::TKBT, 40.0)).unwrap(); // Should overrun up to 44.0 at TKBT

        let overrun_config: PythonBrainConfig = toml::from_str(overrun_config_str).expect("Invalid config string");

        let overrun_result = handle_intention(Intention::Change(ChangeState::FinishMode), &mut info_cache, &mut io_bundle, &overrun_config, &rt, &time).expect("Should succeed");
        assert!(matches!(overrun_result, Some(HeatingMode::HeatUpTo(_))), "Should have overran from finishing mode, got: {:?}", overrun_result);
        temp_handle.send(ModifyState::SetTemps(HashMap::new())).unwrap();

        expect_present(io_bundle.heating_control())
            .try_set_heat_pump(false).expect("Should be able to turn off.");
    }

    // Turn off when both off
    {
        let mut info_cache = InfoCache::create(false, WorkingRange::from_temp_only(WorkingTemperatureRange::from_min_max(30.0, 50.0)));

        let overrun_config_str = r#"
[[overrun_during.slots]]
slot = { type = "Utc", start="11:00:00", end="13:00:05" }
sensor = "TKBT"
temp = 44.0
"#;
        println!("{}", overrun_config_str);
        temp_handle.send(ModifyState::SetTemp(Sensor::TKBT, 44.0)).unwrap();

        let overrun_config: PythonBrainConfig = toml::from_str(overrun_config_str).expect("Invalid config string");

        let overrun_result = handle_intention(Intention::Change(ChangeState::FinishMode), &mut info_cache, &mut io_bundle, &overrun_config, &rt, &time).expect("Should succeed");
        assert!(matches!(overrun_result, Some(HeatingMode::Off)), "Should have turned off after finishing mode and heat pump and wiser of {:?}", overrun_result);
        temp_handle.send(ModifyState::SetTemps(HashMap::new())).unwrap();
    }

    // Go to precirculate if above working temp range
    {
        let mut info_cache = InfoCache::create(true, WorkingRange::from_temp_only(WorkingTemperatureRange::from_min_max(40.0, 50.0)));

        temp_handle.send(ModifyState::SetTemp(Sensor::TKBT, 51.0)).unwrap();

        let pre_ciculate_result = handle_intention(Intention::Change(ChangeState::FinishMode), &mut info_cache, &mut io_bundle, &default_config, &rt, &time).expect("Should succeed");
        assert!(matches!(pre_ciculate_result, Some(HeatingMode::PreCirculate(_))), "Expected circulation but got {:?}", pre_ciculate_result);
    }
}

#[test]
fn test_intention_basic() {
    let time = Utc.from_utc_datetime(&date(2022, 03, 12).and_time(time(12, 30, 00)));

    let heating_control = DummyAllOutputs::default();
    let misc_control = DummyAllOutputs::default();
    let (wiser, _wiser_handle) = wiser::dummy::Dummy::create(&WiserConfig::new(Ipv4Addr::new(0, 0, 0, 0).into(), String::new()));
    let (temp_manager, _temp_handle) = temperatures::dummy::Dummy::create(&());

    let mut io_bundle = IOBundle::new(temp_manager, heating_control, misc_control, wiser);

    let mut info_cache = InfoCache::create(true, WorkingRange::from_temp_only(WorkingTemperatureRange::from_min_max(30.0, 50.0)));

    let rt = Builder::new_multi_thread()
        .worker_threads(1)
        .enable_time()
        .enable_io()
        .build()
        .expect("Expected to be able to make runtime");

    let switch_off_force = handle_intention(Intention::SwitchForce(HeatingMode::Off), &mut info_cache, &mut io_bundle, &Default::default(), &rt, &time).unwrap();
    assert!(matches!(switch_off_force, Some(HeatingMode::Off)), "Forcing switch off should lead to off.");

    let keep_state = handle_intention(Intention::KeepState, &mut info_cache, &mut io_bundle, &Default::default(), &rt, &time).unwrap();
    assert!(keep_state.is_none(), "Keep state should lead to None");
}