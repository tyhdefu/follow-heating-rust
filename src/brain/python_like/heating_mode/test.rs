use std::net::Ipv4Addr;
use chrono::{Date, NaiveDate};
use tokio::runtime::Builder;
use crate::{DummyIO, io, temperatures, wiser, WiserConfig};
use crate::brain::python_like::circulate_heat_pump::StoppingStatus;
use super::*;

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

    fn expect_gpio_present<G>(gpio: &mut Dispatchable<G>) -> &mut G
        where G: GPIOManager {
        if let Dispatchable::Available(gpio) = gpio {
            return gpio;
        }
        panic!("GPIO not available.");
    }

    let config = PythonBrainConfig::default();

    fn print_state(gpio: &impl GPIOManager) {
        let state = gpio.get_pin(HEAT_PUMP_RELAY).unwrap();
        println!("HP GPIO state {:?}", state);

        let state = gpio.get_pin(HEAT_CIRCULATION_PUMP).unwrap();
        println!("CP GPIO state {:?}", state);
    }

    let mut test_transition_between = |mut from: HeatingMode, mut to: HeatingMode| {

        println!("-- Testing {:?} -> {:?} --", from, to);

        {
            println!("- Pre");
            print_state(expect_gpio_present(io_bundle.gpio()));

            from.enter(&config, &rt, &mut io_bundle)?;

            println!("- Init");
            print_state(expect_gpio_present(io_bundle.gpio()));

            let entry_preferences = to.get_entry_preferences().clone();
            let transition_msg = format!("transition {:?} -> {:?}", from, to);

            from.exit_to(&mut to, &mut io_bundle)?;

            {
                let gpio = expect_gpio_present(io_bundle.gpio());

                println!("- Exited");
                print_state(gpio);

                let state = gpio.get_pin(HEAT_PUMP_RELAY).unwrap();
                println!("HP State {:?}", state);
                let on = matches!(state, GPIOState::LOW);

                println!("HP on: {}", on);
                if !entry_preferences.allow_heat_pump_on {
                    assert!(!on, "HP should be off between {}", transition_msg);
                }
                else if on {
                    println!("Leaving on HP correctly.");
                }

                let state = gpio.get_pin(HEAT_CIRCULATION_PUMP).unwrap();
                println!("CP State: {:?}", state);
                let on = matches!(state, GPIOState::LOW);
                println!("CP on: {}", on);
                if !entry_preferences.allow_circulation_pump_on {
                    assert!(!on, "CP should be off between {}", transition_msg);
                }
                else if on {
                    println!("Leaving on CP correctly.");
                }
            }

            to.enter(&config, &rt, &mut io_bundle)?;

        }

        if let HeatingMode::Circulate(_) = to {
            io_bundle.gpio().rob_or_get_now().expect("Should have been able to rob gpio access.");
        }

        // Reset pins.
        let gpio = expect_gpio_present(io_bundle.gpio());
        print_state(gpio);
        gpio.set_pin(HEAT_PUMP_RELAY, &GPIOState::HIGH).expect("Should be able to turn off HP");
        gpio.set_pin(HEAT_CIRCULATION_PUMP, &GPIOState::HIGH).expect("Should be able to turn off CP");

        Ok(())
    };

    test_transition_between(HeatingMode::Off, HeatingMode::On(HeatingOnStatus::default()))?;
    test_transition_between(HeatingMode::On(HeatingOnStatus::default()), HeatingMode::Off)?;
    test_transition_between(HeatingMode::Off, HeatingMode::Circulate(CirculateStatus::Uninitialised))?;
    test_transition_between(HeatingMode::On(HeatingOnStatus::default()), HeatingMode::Circulate(CirculateStatus::Uninitialised))?;
    test_transition_between(HeatingMode::Circulate(CirculateStatus::Stopping(StoppingStatus::stopped())), HeatingMode::Off)?;
    test_transition_between(HeatingMode::Circulate(CirculateStatus::Stopping(StoppingStatus::stopped())), HeatingMode::On(HeatingOnStatus::default()))?;
    test_transition_between(HeatingMode::HeatUpTo(HeatUpTo::new(TargetTemperature::new(Sensor::TKBT, 47.0), Utc::now())), HeatingMode::Off)?;
    Ok(())
}

#[test]
pub fn test() {
    let state = GPIOState::HIGH;
    assert!(matches!(state, GPIOState::HIGH), "Expected High == High");
    assert!(!matches!(state, GPIOState::LOW), "Expected High != Low")
}

#[test]
pub fn test_overrun() {
    let config = PythonBrainConfig::default();

    fn get_adjusted(date: Date<Local>, time: NaiveTime) -> DateTime<Utc> {
        Utc::from_utc_datetime(&Utc, &date.and_time(time).unwrap().naive_utc())
    }

    let datetime = Local::from_local_datetime(&Local, &NaiveDate::from_ymd(2020, 12, 1).and_hms(6, 0, 43)).single().unwrap();
    assert!(get_overrun(datetime, &config).is_none());

    let datetime = Local::from_local_datetime(&Local, &NaiveDate::from_ymd(2020, 03, 2).and_hms(2, 0, 41)).single().unwrap();
    let overrun = get_overrun(datetime, &config);

    if let HeatingMode::HeatUpTo(to) = overrun.unwrap() {
        assert_eq!(to.expire, get_adjusted(datetime.date(), config.overrun_during[0].end));
    }

    let datetime = Local::from_local_datetime(&Local, &NaiveDate::from_ymd(2020, 12, 3).and_hms(11, 2, 0)).single().unwrap();
    assert!(get_overrun(datetime, &config).is_none());

    let datetime = Local::from_local_datetime(&Local, &NaiveDate::from_ymd(2020, 08, 4).and_hms(1, 30, 27)).single().unwrap();
    let overrun = get_overrun(datetime, &config);

    if let HeatingMode::HeatUpTo(to) = overrun.unwrap() {
        assert_eq!(to.expire, get_adjusted(datetime.date(), config.overrun_during[0].end));
    }

    let datetime = Local::from_local_datetime(&Local, &NaiveDate::from_ymd(2020, 11, 4).and_hms(12, 30, 00)).single().unwrap();
    let overrun = get_overrun(datetime, &config);

    if let HeatingMode::HeatUpTo(to) = overrun.unwrap() {
        assert_eq!(to.expire, get_adjusted(datetime.date(), config.overrun_during[1].end));
    }
}