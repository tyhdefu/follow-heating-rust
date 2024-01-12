use crate::brain::python_like::control::devices::Device;
use crate::io::devices::dummy::ActiveDevicesMessage;
use crate::io::dummy::DummyAllOutputs;
use crate::io::dummy_io_bundle::new_dummy_io;
use crate::io::temperatures::dummy::ModifyState::SetTemp;
use crate::io::temperatures::Sensor;
use crate::io::wiser::dummy::ModifyState;
use crate::time_util::mytime::{DummyTimeProvider, TimeProvider};
use crate::{brain, LoggingHandle};
use chrono::{NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Utc};
use log::debug;
use std::time::Duration;
use tokio::runtime::Builder;
use tracing::Subscriber;
use tracing_subscriber::EnvFilter;

const SIMULATION_CONFIG: &'static str = r#"[[overrun_during.slots]]
slot = { type = "Utc", start="04:00:00", end="15:00:05" }
sensor = "TKBT"
temp = 50.0
min_temp = 30.0

[[boost_active_rooms.parts]]
room = "JohnsRoom"
device = "JohnsPhone"
increase = 3.0
"#;

pub fn simulate(logging_handle: LoggingHandle<EnvFilter, impl Subscriber>) {
    let backup_heating_supplier = || DummyAllOutputs::default();
    let (io_bundle, mut io_handle) = new_dummy_io();

    debug!("{}", SIMULATION_CONFIG);

    let config = toml::from_str(SIMULATION_CONFIG).expect("Failed to deserialize config");

    let brain = brain::python_like::PythonBrain::new(config);

    let rt = Builder::new_multi_thread()
        .worker_threads(1)
        .enable_time()
        .enable_io()
        .build()
        .expect("Expected to be able to make runtime");

    //rt.spawn(io::gpio::update_db_with_gpio::run(pool.clone(), receiver));

    //sender.try_send(PinUpdate::new(1, GPIOState::LOW)).unwrap();

    let overrun_time = Utc.from_utc_datetime(
        &NaiveDate::from_ymd_opt(2022, 05, 19)
            .unwrap()
            .and_time(NaiveTime::from_hms_opt(04, 18, 00).unwrap()),
    );
    let no_overrun_time = Utc.from_utc_datetime(
        &NaiveDate::from_ymd_opt(2022, 05, 19)
            .unwrap()
            .and_time(NaiveTime::from_hms_opt(18, 0, 0).unwrap()),
    );

    let time_provider = DummyTimeProvider::new(no_overrun_time);

    println!("Current time {:?}", time_provider.get_utc_time());
    io_handle.send_devices(ActiveDevicesMessage::SetActiveDevices(vec![
        Device::new("StevesPhone".into()),
        Device::new("JohnsPhone".into()),
    ]));

    rt.spawn(async move {
        tokio::time::sleep(Duration::from_secs(5)).await;

        println!("## Set temp to 30C at the bottom.");
        io_handle.send_temps(SetTemp(Sensor::TKBT, 30.0));
        io_handle.send_temps(SetTemp(Sensor::TKTP, 30.0));
        io_handle.send_temps(SetTemp(Sensor::HPRT, 25.0));
        //tokio::time::sleep(Duration::from_secs(20)).await;

        println!("## Set temp to 50C at TKTP.");
        io_handle.send_temps(SetTemp(Sensor::TKTP, 50.5));
        //tokio::time::sleep(Duration::from_secs(30)).await;

        println!("Test TurningOn state");
        io_handle.send_temps(SetTemp(Sensor::TKBT, 50.5)); // Make sure up to finish any heat ups
        tokio::time::sleep(Duration::from_secs(10)).await;
        io_handle.send_temps(SetTemp(Sensor::TKBT, 48.0)); // Then make sure we will turn on.
        io_handle.send_temps(SetTemp(Sensor::HXOR, 30.0));
        io_handle.send_temps(SetTemp(Sensor::HXIF, 32.0));
        io_handle.send_temps(SetTemp(Sensor::HXIR, 31.0));
        tokio::time::sleep(Duration::from_secs(10)).await;
        io_handle.send_wiser(ModifyState::SetHeatingOffTime(
            Utc::now() + chrono::Duration::seconds(1000),
        ));
        //tokio::time::sleep(Duration::from_secs(100)).await;

        println!("## Turning off wiser - expect overrun");
        io_handle.send_wiser(ModifyState::TurnOffHeating);
        //tokio::time::sleep(Duration::from_secs(60)).await;

        println!("## Turning on fake wiser heating");
        tokio::time::sleep(Duration::from_secs(10)).await;
        io_handle.send_temps(SetTemp(Sensor::HPRT, 31.0));
        io_handle.send_wiser(ModifyState::SetHeatingOffTime(
            Utc::now() + chrono::Duration::seconds(1000),
        ));
        /*tokio::time::sleep(Duration::from_secs(90)).await;

        println!("## Set temp to 55C at HXIF/R, expect circulation");
        io_handle.send_temps(SetTemp(Sensor::HXIF, 60.0));
        io_handle.send_temps(SetTemp(Sensor::HXIR, 60.0));
        io_handle.send_temps(SetTemp(Sensor::HXOR, 45.0));
        tokio::time::sleep(Duration::from_secs(60)).await;
        io_handle.send_temps(SetTemp(Sensor::HXIF, 35.0));
        io_handle.send_temps(SetTemp(Sensor::HXIR, 33.0));
        io_handle.send_temps(SetTemp(Sensor::HXOR, 20.0));

        println!("## Turning off fake wiser heating");
        io_handle.send_wiser(ModifyState::TurnOffHeating);
        tokio::time::sleep(Duration::from_secs(60)).await;

        println!("## Turning on fake wiser heating");
        io_handle.send_wiser(ModifyState::SetHeatingOffTime(
            Utc::now() + chrono::Duration::seconds(1000),
        ));
        tokio::time::sleep(Duration::from_secs(30)).await;
        io_handle.send_temps(SetTemp(Sensor::TKBT, 47.0));
        io_handle.send_temps(SetTemp(Sensor::TKTP, 47.0));
        tokio::time::sleep(Duration::from_secs(60)).await;*/

        println!("## Setting HXIF/R to above the turn off temp.");
        io_handle.send_temps(SetTemp(Sensor::HXIF, 60.0));
        io_handle.send_temps(SetTemp(Sensor::HXIR, 60.0));
        io_handle.send_temps(SetTemp(Sensor::HXOR, 60.0));
        io_handle.send_temps(SetTemp(Sensor::TKBT, 61.0));
        tokio::time::sleep(Duration::from_secs(60 * 5 + 30)).await;
        println!("## Now turning back down.");
        io_handle.send_temps(SetTemp(Sensor::HXIF, 32.0));
        io_handle.send_temps(SetTemp(Sensor::HXIR, 32.0));
        tokio::time::sleep(Duration::from_secs(30)).await;

        println!("## Testing Off -> Circulate");
        io_handle.send_wiser(ModifyState::TurnOffHeating);
        tokio::time::sleep(Duration::from_secs(10)).await;
        io_handle.send_temps(SetTemp(Sensor::TKBT, 55.0));
        io_handle.send_wiser(ModifyState::SetHeatingOffTime(
            Utc::now() + chrono::Duration::seconds(1000),
        ));
        tokio::time::sleep(Duration::from_secs(200)).await;
        println!("## Setting HXIF/R to above turn off temp and setting TKBT too low to circulate");
        io_handle.send_temps(SetTemp(Sensor::HXIF, 60.0));
        io_handle.send_temps(SetTemp(Sensor::HXIR, 60.0));
        io_handle.send_temps(SetTemp(Sensor::HXOR, 60.0));
        io_handle.send_temps(SetTemp(Sensor::TKBT, 15.0));
        tokio::time::sleep(Duration::from_secs(60 * 10 + 30)).await;

        println!("## Turning TKTP below desired temp");
        io_handle.send_temps(SetTemp(Sensor::TKTP, 20.0));
        tokio::time::sleep(Duration::from_secs(60)).await;
        println!("## Turning TKTP below desired temp");
        io_handle.send_temps(SetTemp(Sensor::TKTP, 35.0));
        tokio::time::sleep(Duration::from_secs(60)).await;
    });

    let imaginary_handle = rt.spawn(async {});
    crate::main_loop(
        brain,
        io_bundle,
        rt,
        backup_heating_supplier,
        time_provider,
        logging_handle,
        imaginary_handle,
    );

    //sleep(Duration::from_secs(30));
    //println!("Turning off heating.");
    //wiser_handle.send(ModifyState::TurnOffHeating).unwrap();
}
