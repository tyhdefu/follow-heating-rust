use std::fs;
use std::net::Ipv4Addr;
use std::ops::DerefMut;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::sleep;
use std::time::{Duration, Instant};
use chrono::Utc;
use futures::SinkExt;
use sqlx::MySqlPool;
use tokio::runtime::{Builder, Runtime};
use tokio::signal::unix::SignalKind;
use tokio::sync::mpsc::{Receiver, Sender};
use tokio::time::error::Elapsed;
use brain::python_like;
use brain::python_like::config::PythonBrainConfig;
use crate::config::{Config, DatabaseConfig, WiserConfig};
use crate::io::gpio::{GPIOManager, GPIOMode, GPIOState, PinUpdate};
use crate::io::temperatures::database::DBTemperatureManager;
use crate::io::temperatures::{Sensor, TemperatureManager};
use crate::io::wiser::WiserManager;
use io::wiser;
use crate::brain::Brain;
use crate::io::dummy::DummyIO;
use crate::io::{IOBundle, temperatures};
use crate::io::controls::heat_pump::HeatPumpControl;
use crate::io::gpio::sysfs_gpio::SysFsGPIO;
use crate::io::temperatures::dummy::ModifyState::SetTemp;
use crate::io::wiser::dummy::ModifyState;
use crate::python_like::config::try_read_python_brain_config;
use crate::python_like::PythonLikeGPIOManager;
use crate::time::mytime::get_utc_time;
use crate::wiser::hub::WiserHub;

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

    if cfg!(debug_assertions) {
        simulate();
        panic!("Testing.");
    }

    let db_url = make_db_url(config.get_database());
    let pool = futures::executor::block_on(MySqlPool::connect(&db_url))
        .expect(&format!("Failed to connect to {}", db_url));
    let mut temps = DBTemperatureManager::new(pool.clone());
    futures::executor::block_on(temps.retrieve_sensors()).unwrap();
    let cur_temps = futures::executor::block_on(temps.retrieve_temperatures()).expect("Failed to retrieve temperatures");
    println!("{:?}", cur_temps);

    let wiser = wiser::dbhub::DBAndHub::new(pool.clone(), config.get_wiser().get_ip().clone(), config.get_wiser().get_secret().to_owned());

    //let gpio = io::gpio::dummy::Dummy::new();
    let (gpio, pin_update_sender, pin_update_recv) = make_gpio();

    let io_bundle = IOBundle::new(temps, gpio, wiser);

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

    let future = io::gpio::update_db_with_gpio::run(pool.clone(), pin_update_recv);
    rt.spawn(future);

    let backup_gpio_supplier = || make_gpio_using(pin_update_sender);

    main_loop(brain, io_bundle, rt, backup_gpio_supplier);
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

fn make_gpio() -> (SysFsGPIO, Sender<PinUpdate>, Receiver<PinUpdate>) {
    let (tx, rx) = tokio::sync::mpsc::channel(5);

    let gpio = make_gpio_using(tx.clone());
    //let gpio = io::gpio::dummy::Dummy::new();
    return (gpio, tx, rx);
}

fn make_gpio_using(sender: Sender<PinUpdate>) -> SysFsGPIO {
    let mut gpio = io::gpio::sysfs_gpio::SysFsGPIO::new(sender);
    gpio.setup(io::controls::heat_circulation_pump::HEAT_CIRCULATION_PUMP, &GPIOMode::Output);
    gpio.setup(io::controls::heat_pump::HEAT_PUMP_RELAY, &GPIOMode::Output);
    gpio.setup(io::controls::immersion_heater::IMMERSION_HEATER, &GPIOMode::Output);
    gpio
}
// 3600
fn test_pulsing<G>(mut gpio: G)
    where G: GPIOManager {

    let args: Vec<String> = std::env::args().collect();
    let arg = args.get(1);
    if let None = arg {
        panic!("You must provide a pulse time in ms.");
    }
    let millis: u64 = arg.unwrap().parse()
        .expect(&format!("Argument should be a number, the sleep time in milliseconds, Got {}", arg.unwrap()));

    let delay = Duration::from_millis(millis);
    let between_pulse = Duration::from_secs(20);
    println!("Doing GPIO pulsing (starting in 1 seconds) of delay {}ms", millis);
    sleep(Duration::from_secs(1));
    println!("Current state: {:?}", gpio.try_get_heat_pump().unwrap());

    loop {
        println!("Turning off");
        let before = Instant::now();
        gpio.try_set_heat_pump(false).unwrap();
        sleep(delay);
        gpio.try_set_heat_pump(true).unwrap();
        println!("Elapsed: {}ms", before.elapsed().as_millis());
        println!("Turned on - Waiting {} seconds before turning back off", between_pulse.as_secs());
        sleep(between_pulse);
    }
}

fn simulate() {
    //let (sender, receiver) = tokio::sync::mpsc::channel(5);

    //let pool = futures::executor::block_on(MySqlPool::connect(&format!("mysql://{}:{}@localhost:{}/{}", "pi", "****", 3309, "heating")))
    //    .expect(&format!("Failed to connect to"));

    let gpios = io::gpio::dummy::Dummy::new();
    let (wiser, wiser_handle) = wiser::dummy::Dummy::create(&WiserConfig::new(Ipv4Addr::new(0, 0, 0, 0).into(), String::new()));
    let (temp_manager, temp_handle) = temperatures::dummy::Dummy::create(&());

    let backup_gpio_supplier = || io::gpio::dummy::Dummy::new();

    let io_bundle = IOBundle::new(temp_manager, gpios, wiser);

    let brain = brain::python_like::PythonBrain::default();

    let rt = Builder::new_multi_thread()
        .worker_threads(1)
        .enable_time()
        .enable_io()
        .build()
        .expect("Expected to be able to make runtime");

    //rt.spawn(io::gpio::update_db_with_gpio::run(pool.clone(), receiver));

    //sender.try_send(PinUpdate::new(1, GPIOState::LOW)).unwrap();

    rt.spawn(async move {
        tokio::time::sleep(Duration::from_secs(5)).await;
        println!("Current time {:?}", get_utc_time());

        println!("## Set temp to 30C at the bottom.");
        temp_handle.send(SetTemp(Sensor::TKBT, 30.0)).unwrap();
        temp_handle.send(SetTemp(Sensor::TKTP, 30.0)).unwrap();
        temp_handle.send(SetTemp(Sensor::HPRT, 25.0)).unwrap();
        tokio::time::sleep(Duration::from_secs(30)).await;

        println!("## Set temp to 50C at TKTP.");
        temp_handle.send(SetTemp(Sensor::TKTP, 50.5)).unwrap();
        tokio::time::sleep(Duration::from_secs(60)).await;

        println!("Test TurningOn state");
        temp_handle.send(SetTemp(Sensor::TKBT, 50.5)).unwrap(); // Make sure up to finish any heat ups
        tokio::time::sleep(Duration::from_secs(10)).await;
        temp_handle.send(SetTemp(Sensor::TKBT, 48.0)).unwrap(); // Then make sure we will turn on.
        tokio::time::sleep(Duration::from_secs(10)).await;
        wiser_handle.send(ModifyState::SetHeatingOffTime(Utc::now() + chrono::Duration::seconds(1000))).unwrap();
        tokio::time::sleep(Duration::from_secs(100)).await;
        wiser_handle.send(ModifyState::TurnOffHeating).unwrap();

        println!("## Turning on fake wiser heating");
        tokio::time::sleep(Duration::from_secs(10)).await;
        temp_handle.send(SetTemp(Sensor::HPRT, 31.0)).unwrap();
        wiser_handle.send(ModifyState::SetHeatingOffTime(Utc::now() + chrono::Duration::seconds(1000))).unwrap();
        tokio::time::sleep(Duration::from_secs(90)).await;

        println!("## Turning off fake wiser heating");
        wiser_handle.send(ModifyState::TurnOffHeating).unwrap();
        tokio::time::sleep(Duration::from_secs(60)).await;

        println!("## Turning on fake wiser heating");
        wiser_handle.send(ModifyState::SetHeatingOffTime(Utc::now() + chrono::Duration::seconds(1000))).unwrap();
        tokio::time::sleep(Duration::from_secs(30)).await;
        temp_handle.send(SetTemp(Sensor::TKBT, 47.0)).unwrap();
        temp_handle.send(SetTemp(Sensor::TKTP, 47.0)).unwrap();
        tokio::time::sleep(Duration::from_secs(60)).await;

        println!("## Setting TKBT to above the turn off temp.");
        temp_handle.send(SetTemp(Sensor::TKBT, 50.0)).unwrap();
        temp_handle.send(SetTemp(Sensor::TKTP, 50.0)).unwrap();
        tokio::time::sleep(Duration::from_secs(60 * 8 + 30)).await;
        println!("## Now turning back down.");
        temp_handle.send(SetTemp(Sensor::TKBT, 32.0)).unwrap();
        tokio::time::sleep(Duration::from_secs(30)).await;

        println!("## Testing Off -> Circulate");
        wiser_handle.send(ModifyState::TurnOffHeating).unwrap();
        tokio::time::sleep(Duration::from_secs(10)).await;
        temp_handle.send(SetTemp(Sensor::TKBT, 50.0)).unwrap();
        wiser_handle.send(ModifyState::SetHeatingOffTime(Utc::now() + chrono::Duration::seconds(1000))).unwrap();
        tokio::time::sleep(Duration::from_secs(60)).await;

        println!("## Turning TKTP below desired temp");
        temp_handle.send(SetTemp(Sensor::TKTP, 20.0)).unwrap();
        tokio::time::sleep(Duration::from_secs(60)).await;
        println!("## Turning TKTP below desired temp");
        temp_handle.send(SetTemp(Sensor::TKTP, 35.0)).unwrap();
        tokio::time::sleep(Duration::from_secs(60)).await;
    });

    main_loop(brain, io_bundle, rt, backup_gpio_supplier);

    //sleep(Duration::from_secs(30));
    //println!("Turning off heating.");
    //wiser_handle.send(ModifyState::TurnOffHeating).unwrap();
}

fn make_db_url(db_config: &DatabaseConfig) -> String {
    format!("mysql://{}:{}@localhost:{}/{}", db_config.get_user(), db_config.get_password(), db_config.get_port(), db_config.get_database())
}

fn main_loop<B, T, G, W, F>(mut brain: B, mut io_bundle: IOBundle<T, G, W>, rt: Runtime, backup_gpio_supplier: F)
    where
        B: Brain,
        T: TemperatureManager,
        G: PythonLikeGPIOManager + Send + 'static,
        W: WiserManager,
        F: FnOnce() -> G {

    let x = rt.block_on(io_bundle.wiser().get_wiser_hub().get_data());
    println!("Result {:?}", x);


    let (signal_send, mut signal_recv) = tokio::sync::mpsc::channel(5);

    #[cfg(target_family = "unix")]
    {
        println!("Subscribing to signals.");
        subscribe_signal(&rt, SignalKind::interrupt(), signal_send.clone(), Signal::Stop);
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

        let result = brain.run(&rt, &mut io_bundle);
        if result.is_err() {
            let err = result.unwrap_err();
            println!("Brain Failure: {:?}", err);
            // TODO: Handle corrective actions.
            println!("Shutting down.");
            shutdown_using_backup(rt, io_bundle, backup_gpio_supplier);
            println!("Done.");
            return;
        }
        if let Some(signal) = rt.block_on(wait_or_get_signal(&mut signal_recv))  {
            println!("Received signal to {:?}", signal);
            match signal {
                Signal::Stop => {
                    println!("Stopping safely...");
                    shutdown_using_backup(rt, io_bundle, backup_gpio_supplier);
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

fn shutdown_using_backup<T,G,W,F>(rt: Runtime, mut io_bundle: IOBundle<T,G,W>, backup_gpio_supplier: F)
    where G: GPIOManager,
        F: FnOnce() -> G,
        T: TemperatureManager,
        W: WiserManager {
    if let Ok(gpio) = io_bundle.gpio().rob_or_get_now() {
        shutdown(rt, gpio.deref_mut())
    } else {
        let mut gpio = backup_gpio_supplier();
        shutdown(rt, &mut gpio)
    }
}

fn shutdown<G>(rt: Runtime, gpio: &mut G)
    where G: PythonLikeGPIOManager {
    let result = gpio.try_set_heat_pump(false);
    if result.is_err() {
        eprintln!("FAILED TO SHUTDOWN HEAT PUMP: {:?}. It may still be on", result.unwrap_err());
    }
    let result = gpio.try_set_heat_circulation_pump(false);
    if result.is_err() {
        eprintln!("FAILED TO SHUTDOWN HEAT CIRCULATION PUMP: {:?}. It may still be on", result.unwrap_err());
    }
    let result = gpio.try_set_immersion_heater(false);
    if result.is_err() {
        eprintln!("FAILED TO SHUTDOWN IMMERSION HEATER: {:?}. It may still be on", result.unwrap_err());
    }
    println!("Waiting for database inserts to be processed.");
    // TODO: Shutdown updater thread.
    rt.shutdown_timeout(Duration::from_millis(1000));
}
