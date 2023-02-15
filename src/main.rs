use std::borrow::BorrowMut;
use std::{fs, panic};
use std::ops::DerefMut;
use std::time::Duration;
use chrono::{NaiveDate, NaiveTime, TimeZone, Utc};
use sqlx::MySqlPool;
use tokio::runtime::{Builder, Runtime};
use tokio::signal::unix::SignalKind;
use tokio::sync::mpsc::{Receiver, Sender};
use brain::python_like;
use brain::python_like::config::PythonBrainConfig;
use crate::config::{Config, DatabaseConfig};
use crate::io::devices::DevicesFromFile;
use crate::io::dummy_io_bundle::new_dummy_io;
use crate::io::gpio::{GPIOError, GPIOManager, GPIOMode, GPIOState, PinUpdate};
use crate::io::temperatures::database::DBTemperatureManager;
use crate::io::temperatures::{Sensor, TemperatureManager};
use crate::io::wiser::WiserManager;
use io::wiser;
use crate::brain::{Brain, BrainFailure, CorrectiveActions};
use crate::io::dummy::{DummyAllOutputs};
use crate::io::{IOBundle};
use crate::io::controls::heating_impl::GPIOHeatingControl;
use crate::io::controls::misc_impl::MiscGPIOControls;
use crate::io::gpio::sysfs_gpio::SysFsGPIO;
use crate::io::temperatures::dummy::ModifyState::SetTemp;
use crate::io::wiser::dummy::ModifyState;
use crate::python_like::config::try_read_python_brain_config;
use crate::python_like::control::heating_control::HeatingControl;
use crate::python_like::control::misc_control::MiscControls;
use crate::time::mytime::{DummyTimeProvider, RealTimeProvider, TimeProvider};
use crate::wiser::hub::WiserHub;
use crate::brain::python_like::control::misc_control::ImmersionHeaterControl;

mod io;
mod config;
mod brain;
mod math;
mod time;

const CONFIG_FILE: &str = "follow_heating.toml";

fn check_config() {
    let config = fs::read_to_string(CONFIG_FILE)
        .expect("Unable to read test config file. Is it missing?");
    let _config: Config = toml::from_str(&*config)
        .expect("Error reading test config file");

    try_read_python_brain_config().expect("Failed to read python brain config.");
}

fn main() {

    let args: Vec<String> = std::env::args().collect();
    if args.len() == 1 && args[0] == "check-config" {
        if args[0] == "check-config" {
            check_config();
            println!("Config OK!");
        }
        println!("Unrecognized argument: {}, run with no args to run normally.", args.len());
        return;
    }

    println!("Preparing...");

    let config = fs::read_to_string(CONFIG_FILE)
        .expect("Unable to read test config file. Is it missing?");
    let config: Config = toml::from_str(&*config)
        .expect("Error reading test config file");

    let default_hook = panic::take_hook();
    panic::set_hook(Box::new(move |panic| {
        eprintln!("PANICKED: {:?}: Shutting down", panic);
        let (send, _recv) = tokio::sync::mpsc::channel(1);
        match make_controls(send) {
            Ok((mut heating_controls, mut misc_controls)) => {
                shutdown_heating(&mut heating_controls);
                shutdown_misc(&mut misc_controls);
            },
            Err(e) => {
                eprintln!("Failed to get access to controls, anything could be on/off: {}", e);
            }
        }
        eprintln!("Warning: Unlikely to have recorded state correctly in database.");
        default_hook(panic);
    }));

    if cfg!(debug_assertions) {
        simulate();
        panic!("Testing.");
    }

    // Read brain config.
    let python_brain_config = read_python_brain_config();

    println!("python brain config {:?}", &python_brain_config);

    let brain = brain::python_like::PythonBrain::new(python_brain_config);

    let rt = Builder::new_multi_thread()
        .worker_threads(3)
        .enable_time()
        .enable_io()
        .build()
        .expect("Expected to be able to make runtime");

    let db_url = make_db_url(config.get_database());
    let pool = futures::executor::block_on(MySqlPool::connect(&db_url))
        .expect(&format!("Failed to connect to {}", db_url));

    let (io_bundle, pin_update_sender, pin_update_recv) = make_io_bundle(config, pool.clone()).expect("Failed to make io bundle.");

    let backup = make_heating_control(pin_update_sender.clone()).expect("Failed to create backup");
    let backup_supplier = || backup;

    let future = io::gpio::update_db_with_gpio::run(pool.clone(), pin_update_recv);
    rt.spawn(future);

    main_loop(brain, io_bundle, rt, backup_supplier, RealTimeProvider::default());
}

fn read_python_brain_config() -> PythonBrainConfig {
    match python_like::config::try_read_python_brain_config() {
        None => {
            eprintln!("Using default config as couldn't read python brain config");
            PythonBrainConfig::default()
        }
        Some(config) => config
    }
}

fn make_io_bundle(config: Config, pool: MySqlPool) -> Result<(IOBundle, Sender<PinUpdate>, Receiver<PinUpdate>), Box<BrainFailure>> {
    let mut temps = DBTemperatureManager::new(pool.clone());
    futures::executor::block_on(temps.retrieve_sensors()).unwrap();
    let cur_temps = futures::executor::block_on(temps.retrieve_temperatures()).expect("Failed to retrieve temperatures");
    println!("{:?}", cur_temps);

    let wiser = wiser::dbhub::DBAndHub::new(pool.clone(), config.get_wiser().get_ip().clone(), config.get_wiser().get_secret().to_owned());

    let (pin_update_sender, pin_update_recv) = tokio::sync::mpsc::channel(5);
    let (heating_controls, misc_controls) = make_controls(pin_update_sender.clone())?;

    let active_devices = DevicesFromFile::create(config.get_devices());

    Ok((IOBundle::new(temps, heating_controls, misc_controls, wiser, active_devices), pin_update_sender, pin_update_recv))
}

fn make_controls(sender: Sender<PinUpdate>) -> Result<(impl HeatingControl, impl MiscControls), BrainFailure> {
    let heating_controls = make_heating_control(sender.clone())
        .map_err(|e| brain_fail!(format!("Failed to setup heating controls: {:?}", e)))?;
    let misc_controls = make_misc_control(sender.clone())
        .map_err(|e| brain_fail!(format!("Failed to setup misc controls: {:?}", e)))?;

    Ok((heating_controls, misc_controls))
}

const HEAT_PUMP_RELAY: usize = 26;
const HEAT_CIRCULATION_RELAY: usize = 5;
const IMMERSION_HEATER_RELAY: usize = 6;
const WISER_POWER_RELAY: usize = 13;

fn make_heating_control(sender: Sender<PinUpdate>) -> Result<impl HeatingControl, GPIOError> {
    let control = GPIOHeatingControl::create(
        HEAT_PUMP_RELAY,
        HEAT_CIRCULATION_RELAY,
        sender
    )?;
    Ok(control)
}

fn make_misc_control(sender: Sender<PinUpdate>) -> Result<impl MiscControls, GPIOError> {
    let control = MiscGPIOControls::create(
        IMMERSION_HEATER_RELAY,
        WISER_POWER_RELAY,
        sender
    )?;
    Ok(control)
}

fn simulate() {
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

    main_loop(brain, io_bundle, rt, backup_heating_supplier, time_provider);

    //sleep(Duration::from_secs(30));
    //println!("Turning off heating.");
    //wiser_handle.send(ModifyState::TurnOffHeating).unwrap();
}

fn make_db_url(db_config: &DatabaseConfig) -> String {
    format!("mysql://{}:{}@localhost:{}/{}", db_config.get_user(), db_config.get_password(), db_config.get_port(), db_config.get_database())
}

fn main_loop<B, H, F>(mut brain: B, mut io_bundle: IOBundle, rt: Runtime, backup_supplier: F, time_provider: impl TimeProvider)
    where
        B: Brain,
        H: HeatingControl,
        F: FnOnce() -> H {

    let x = rt.block_on(io_bundle.wiser().get_wiser_hub().get_data());
    println!("Result {:?}", x);


    let (signal_send, mut signal_recv) = tokio::sync::mpsc::channel(5);

    #[cfg(target_family = "unix")]
    {
        println!("Subscribing to signals.");
        subscribe_signal(&rt, SignalKind::interrupt(), signal_send.clone(), Signal::Stop);
        subscribe_signal(&rt, SignalKind::terminate(), signal_send.clone(), Signal::Stop);
        subscribe_signal(&rt, SignalKind::user_defined1(), signal_send.clone(), Signal::ReloadConfig);
    }
    #[cfg(not(target_family = "unix"))]
    {
        let signal_send = signal_send.clone();
        ctrlc::set_handler(move || {
            println!("Received termination signal.");
            signal_send.blocking_send(Signal::Stop).unwrap();
        }).expect("Failed to attach kill handler.");
    }

    //let mut interval = tokio::time::interval(Duration::from_secs(2));
    //interval.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut i = 0;
    println!("Beginning main loop.");
    loop {
        i += 1;
        if i % 6 == 0 {
            println!("Still alive..")
        }

        let result = brain.run(&rt, &mut io_bundle, &time_provider);
        if let Err(err) = result {
            println!("Brain Failure: {}", err);
            // TODO: Handle corrective actions.
            println!("Shutting down.");
            let _ = panic::take_hook(); // Remove our custom panic hook.
            shutdown_using_backup(rt, io_bundle, backup_supplier);
            panic!("Had brain failure: see above.");
        }
        if let Some(signal) = rt.block_on(wait_or_get_signal(&mut signal_recv))  {
            println!("Received signal to {:?}", signal);
            match signal {
                Signal::Stop => {
                    println!("Stopping safely...");
                    shutdown_using_backup(rt, io_bundle, backup_supplier);
                    // TODO: Check for important stuff going on.
                    println!("Stopped safely.");
                    return;
                }
                Signal::ReloadConfig => {
                    println!("Reloading config");
                    brain.reload_config();
                    println!("Reloading config complete")
                }
            }
        }
    }
}

fn subscribe_signal(rt: &Runtime, kind: SignalKind, sender: Sender<Signal>, signal: Signal) {
    rt.spawn(async move {
        let mut recv = tokio::signal::unix::signal(kind).expect("Failed to get signal handler");
        while let Some(()) = recv.recv().await {
            sender.send(signal.clone()).await.unwrap();
        }
    });
}

#[derive(Debug, Clone)]
enum Signal {
    Stop,
    ReloadConfig,
}

async fn wait_or_get_signal(recv: &mut Receiver<Signal>) -> Option<Signal> {
    let result = tokio::time::timeout(Duration::from_secs(10), recv.recv()).await;
    match result {
        Ok(None) => None, // Channel closed
        Ok(Some(signal)) => Some(signal),
        Err(_) => None, // Timed out.
    }
}

fn shutdown_using_backup<F, H>(rt: Runtime, mut io_bundle: IOBundle, backup_supplier: F)
    where H: HeatingControl,
        F: FnOnce() -> H {

    shutdown_misc(io_bundle.misc_controls());

    if let Ok(heating_control) = io_bundle.heating_control().rob_or_get_now() {
        shutdown_heating(heating_control.deref_mut().borrow_mut())
    } else {
        let mut gpio = backup_supplier();
        shutdown_heating(&mut gpio)
    }

    println!("Waiting for database inserts to be processed.");
    // TODO: Shutdown updater thread.
    rt.shutdown_timeout(Duration::from_millis(1000));
}

fn shutdown_misc(misc_controls: &mut dyn MiscControls) {
    if let Err(e) = misc_controls.try_set_immersion_heater(false) {
        eprintln!("FAILED TO SHUTDOWN IMMERSION HEATER: {:?}. It may still be on", e);
    }
    if let Err(e) = misc_controls.try_set_wiser_power(true) {
        eprintln!("FAILED TO TURN BACK ON WISER POWER: {:?}. It may be off.", e);
    }
}

fn shutdown_heating(heating_control: &mut dyn HeatingControl) {
    if let Err(e) = heating_control.try_set_heat_pump(false) {
        eprintln!("FAILED TO SHUTDOWN HEAT PUMP: {:?}. It may still be on", e);
    }
    if let Err(e) = heating_control.try_set_heat_circulation_pump(false) {
        eprintln!("FAILED TO SHUTDOWN HEAT CIRCULATION PUMP: {:?}. It may still be on", e);
    }
}

