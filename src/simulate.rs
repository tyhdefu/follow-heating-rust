use tokio::runtime::Builder;
use chrono::{NaiveDate, NaiveTime, TimeZone, Utc};
use std::time::Duration;
use crate::brain;
use crate::io::dummy::DummyAllOutputs;
use crate::io::dummy_io_bundle::new_dummy_io;
use crate::io::temperatures::dummy::ModifyState::SetTemp;
use crate::io::temperatures::Sensor;
use crate::io::wiser::dummy::ModifyState;
use crate::time_util::mytime::{DummyTimeProvider, TimeProvider};

pub fn simulate() {
    let backup_heating_supplier = || DummyAllOutputs::default();
    let (io_bundle, mut io_handle) = new_dummy_io();

    let brain = brain::python_like::PythonBrain::default();

    let rt = Builder::new_multi_thread()
        .worker_threads(1)
        .enable_time()
        .enable_io()
        .build()
        .expect("Expected to be able to make runtime");

    //rt.spawn(io::gpio::update_db_with_gpio::run(pool.clone(), receiver));

    //sender.try_send(PinUpdate::new(1, GPIOState::LOW)).unwrap();

    let time_provider = DummyTimeProvider::new(Utc.from_utc_datetime(
        &NaiveDate::from_ymd_opt(2022, 05, 19).unwrap()
            .and_time(NaiveTime::from_hms_opt(12, 00, 00).unwrap())
    ));

    println!("Current time {:?}", time_provider.get_utc_time());

    rt.spawn(async move {
        tokio::time::sleep(Duration::from_secs(5)).await;

        println!("## Set temp to 30C at the bottom.");
        io_handle.send_temps(SetTemp(Sensor::TKBT, 30.0));
        io_handle.send_temps(SetTemp(Sensor::TKTP, 30.0));
        io_handle.send_temps(SetTemp(Sensor::HPRT, 25.0));
        tokio::time::sleep(Duration::from_secs(20)).await;

        println!("## Set temp to 50C at TKTP.");
        io_handle.send_temps(SetTemp(Sensor::TKTP, 50.5));
        tokio::time::sleep(Duration::from_secs(30)).await;

        println!("Test TurningOn state");
        io_handle.send_temps(SetTemp(Sensor::TKBT, 50.5)); // Make sure up to finish any heat ups
        tokio::time::sleep(Duration::from_secs(10)).await;
        io_handle.send_temps(SetTemp(Sensor::TKBT, 48.0)); // Then make sure we will turn on.
        tokio::time::sleep(Duration::from_secs(10)).await;
        io_handle.send_wiser(ModifyState::SetHeatingOffTime(Utc::now() + chrono::Duration::seconds(1000)));
        tokio::time::sleep(Duration::from_secs(100)).await;

        println!("## Turning off wiser - expect overrun");
        io_handle.send_wiser(ModifyState::TurnOffHeating);
        tokio::time::sleep(Duration::from_secs(60)).await;

        println!("## Turning on fake wiser heating");
        tokio::time::sleep(Duration::from_secs(10)).await;
        io_handle.send_temps(SetTemp(Sensor::HPRT, 31.0));
        io_handle.send_wiser(ModifyState::SetHeatingOffTime(Utc::now() + chrono::Duration::seconds(1000)));
        tokio::time::sleep(Duration::from_secs(90)).await;

        println!("## Turning off fake wiser heating");
        io_handle.send_wiser(ModifyState::TurnOffHeating);
        tokio::time::sleep(Duration::from_secs(60)).await;

        println!("## Turning on fake wiser heating");
        io_handle.send_wiser(ModifyState::SetHeatingOffTime(Utc::now() + chrono::Duration::seconds(1000)));
        tokio::time::sleep(Duration::from_secs(30)).await;
        io_handle.send_temps(SetTemp(Sensor::TKBT, 47.0));
        io_handle.send_temps(SetTemp(Sensor::TKTP, 47.0));
        tokio::time::sleep(Duration::from_secs(60)).await;

        println!("## Setting TKBT to above the turn off temp.");
        io_handle.send_temps(SetTemp(Sensor::TKBT, 50.0));
        io_handle.send_temps(SetTemp(Sensor::TKTP, 50.0));
        tokio::time::sleep(Duration::from_secs(60 * 8 + 30)).await;
        println!("## Now turning back down.");
        io_handle.send_temps(SetTemp(Sensor::TKBT, 32.0));
        tokio::time::sleep(Duration::from_secs(30)).await;

        println!("## Testing Off -> Circulate");
        io_handle.send_wiser(ModifyState::TurnOffHeating);
        tokio::time::sleep(Duration::from_secs(10)).await;
        io_handle.send_temps(SetTemp(Sensor::TKBT, 50.0));
        io_handle.send_wiser(ModifyState::SetHeatingOffTime(Utc::now() + chrono::Duration::seconds(1000)));
        tokio::time::sleep(Duration::from_secs(60)).await;

        println!("## Turning TKTP below desired temp");
        io_handle.send_temps(SetTemp(Sensor::TKTP, 20.0));
        tokio::time::sleep(Duration::from_secs(60)).await;
        println!("## Turning TKTP below desired temp");
        io_handle.send_temps(SetTemp(Sensor::TKTP, 35.0));
        tokio::time::sleep(Duration::from_secs(60)).await;
    });

    crate::main_loop(brain, io_bundle, rt, backup_heating_supplier, time_provider);

    //sleep(Duration::from_secs(30));
    //println!("Turning off heating.");
    //wiser_handle.send(ModifyState::TurnOffHeating).unwrap();
}
