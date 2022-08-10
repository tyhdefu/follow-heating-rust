use std::net::Ipv4Addr;
use chrono::{Date, NaiveDate, NaiveDateTime};
use tokio::runtime::Builder;
use crate::{DummyIO, GPIOState, io, temperatures, wiser, WiserConfig};
use crate::brain::python_like::circulate_heat_pump::StoppingStatus;
use crate::io::controls::{heat_circulation_pump::HeatCirculationPumpControl,
                          heat_pump::HeatPumpControl};
use crate::python_like::heating_mode;
use crate::python_like::heatupto::{HeatUpEnd, HeatUpTo};
use crate::python_like::overrun_config::OverrunConfig;
use crate::temperatures::dummy::ModifyState;

use super::*;
struct CleanupHandle<'a, T, G, W>
    where
        T: TemperatureManager,
        G: GPIOManager + Send + 'static,
        W: WiserManager {
    io_bundle: &'a mut IOBundle<T, G, W>,
    heating_mode: HeatingMode,
}

impl<'a, T, G, W> CleanupHandle<'a, T, G, W>
    where
        T: TemperatureManager,
        G: GPIOManager + Send + 'static,
        W: WiserManager {
    pub fn new(io_bundle: &'a mut IOBundle<T, G, W>, heating_mode: HeatingMode) -> Self {
        Self {
            io_bundle,
            heating_mode,
        }
    }

    pub fn get_io_bundle(&mut self) -> &mut IOBundle<T, G, W> {
        self.io_bundle
    }

    pub fn get_heating_mode(&mut self) -> &mut HeatingMode {
        &mut self.heating_mode
    }

    pub fn update(&mut self, shared_data: &mut SharedData, runtime: &Runtime, config: &PythonBrainConfig) -> Result<Option<HeatingMode>, BrainFailure> {
        self.heating_mode.update(shared_data, runtime, config, self.io_bundle)
    }
}

impl<T, G, W> Drop for CleanupHandle<'_, T, G, W>
    where
        T: TemperatureManager,
        G: GPIOManager + Send + 'static,
        W: WiserManager {
    fn drop(&mut self) {
        if let HeatingMode::Circulate(_) = self.heating_mode {
            self.io_bundle.gpio().rob_or_get_now().expect("Should have been able to rob gpio access.");
        };

        // Reset pins.
        let gpio = expect_gpio_present(self.io_bundle.gpio());
        print_state(gpio);
        gpio.try_set_heat_pump(false).expect("Should be able to turn off HP");
        gpio.try_set_heat_circulation_pump(false).expect("Should be able to turn off CP");
    }
}

fn expect_gpio_present<G>(gpio: &mut Dispatchable<G>) -> &mut G
    where G: PythonLikeGPIOManager {
    if let Dispatchable::Available(gpio) = gpio {
        return gpio;
    }
    panic!("GPIO not available.");
}

fn print_state(gpio: &impl PythonLikeGPIOManager) {
    let state = gpio.try_get_heat_pump().unwrap();
    println!("HP GPIO state {:?}", state);

    let state = gpio.try_get_heat_circulation_pump().unwrap();
    println!("CP GPIO state {:?}", state);
}

#[test]
pub fn test_transitions() -> Result<(), BrainFailure> {
    let gpios = io::gpio::dummy::Dummy::new();
    let (wiser, wiser_handle) = wiser::dummy::Dummy::create(&WiserConfig::new(Ipv4Addr::new(0, 0, 0, 0).into(), String::new()));
    let (temp_manager, temp_handle) = temperatures::dummy::Dummy::create(&());

    let mut io_bundle = IOBundle::new(temp_manager, gpios, wiser);

    let rt = Builder::new_multi_thread()
        .worker_threads(1)
        .enable_time()
        .enable_io()
        .build()
        .expect("Expected to be able to make runtime");

    let config = PythonBrainConfig::default();

    let mut shared_data = SharedData::new(FallbackWorkingRange::new(config.get_default_working_range().clone()));

    fn test_transition_fn<'a, T, G, W>(mut from: HeatingMode, mut to: HeatingMode, config: &PythonBrainConfig, rt: &Runtime, io_bundle: &'a mut IOBundle<T, G, W>) -> Result<CleanupHandle<'a,T,G,W>, BrainFailure>
        where
            T: TemperatureManager,
            G: GPIOManager + Send + 'static,
            W: WiserManager, {
        println!("-- Testing {:?} -> {:?} --", from, to);

        println!("- Pre");
        print_state(expect_gpio_present(io_bundle.gpio()));

        from.enter(&config, &rt, io_bundle)?;

        println!("- Init");
        print_state(expect_gpio_present(io_bundle.gpio()));

        let entry_preferences = to.get_entry_preferences().clone();
        let transition_msg = format!("transition {:?} -> {:?}", from, to);

        from.exit_to(&to, io_bundle)?;

        {
            let gpio = expect_gpio_present(io_bundle.gpio());

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
            let gpio = expect_gpio_present(handle.get_io_bundle().gpio());
            assert_eq!(gpio.try_get_heat_pump()?, true, "HP should be on");
            assert_eq!(gpio.try_get_heat_circulation_pump()?, false, "CP should be off");
        }

        println!("Updating state.");
        temp_handle.send(ModifyState::SetTemp(Sensor::HPRT, 35.0)).unwrap();
        temp_handle.send(ModifyState::SetTemp(Sensor::TKBT, 35.0)).unwrap();
        handle.update(&mut shared_data, &rt, &config).unwrap();
        {
            let gpio = expect_gpio_present(handle.get_io_bundle().gpio());
            assert_eq!(gpio.try_get_heat_pump()?, true, "HP should be on");
            assert_eq!(gpio.try_get_heat_circulation_pump()?, true, "CP should be on");
        }
    }

    let mut test_transition_between = |mut from: HeatingMode, mut to: HeatingMode| {
        test_transition_fn(from, to, &config, &rt, &mut io_bundle).map(|_| ())
    };

    test_transition_between(HeatingMode::On(HeatingOnStatus::default()), HeatingMode::Off)?;
    test_transition_between(HeatingMode::Off, HeatingMode::Circulate(CirculateStatus::Uninitialised))?;
    test_transition_between(HeatingMode::On(HeatingOnStatus::default()), HeatingMode::Circulate(CirculateStatus::Uninitialised))?;
    test_transition_between(HeatingMode::Circulate(CirculateStatus::Stopping(StoppingStatus::stopped())), HeatingMode::Off)?;
    test_transition_between(HeatingMode::Circulate(CirculateStatus::Stopping(StoppingStatus::stopped())), HeatingMode::On(HeatingOnStatus::default()))?;
    test_transition_between(HeatingMode::HeatUpTo(HeatUpTo::from_time(TargetTemperature::new(Sensor::TKBT, 47.0), Utc::now())), HeatingMode::Off)?;
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
    let config_str = std::fs::read_to_string("test/test_overrun_scenarios.toml").expect("Failed to read config file.");
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

    let mode = heating_mode::get_heatup_while_off(datetime, &overrun_config, &temps);
    println!("Mode: {:?}", mode);
    assert!(mode.is_some());
    if let HeatingMode::HeatUpTo(heat_up_to) = mode.unwrap() {
        assert_eq!(heat_up_to.get_target().sensor, Sensor::TKBT);
        assert_eq!(heat_up_to.get_target().temp, 49.0)
    }
    else {
        panic!("Should have been heat up to mode.")
    }

    temps.insert(Sensor::TKTP, 52.0);
    temps.insert(Sensor::TKBT, 46.0);
    let mode = heating_mode::get_heatup_while_off(datetime, &overrun_config, &temps);
    println!("Mode: {:?}", mode);
    assert!(mode.is_none());
}