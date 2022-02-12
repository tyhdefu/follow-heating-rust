use std::fs;
use std::net::Ipv4Addr;
use std::ops::DerefMut;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::sleep;
use std::time::{Duration, Instant};
use chrono::Utc;
use sqlx::MySqlPool;
use tokio::runtime::{Builder, Runtime};
use tokio::sync::mpsc::{Receiver, Sender};
use tokio::time::{interval_at, MissedTickBehavior};
use crate::config::{Config, DatabaseConfig, WiserConfig};
use crate::io::gpio::{GPIOManager, GPIOMode, GPIOState, PinUpdate};
use crate::io::temperatures::database::DBTemperatureManager;
use crate::io::temperatures::{Sensor, TemperatureManager};
use crate::io::wiser::WiserManager;
use io::wiser;
use crate::brain::Brain;
use crate::brain::python_like::HEAT_PUMP_RELAY;
use crate::io::dummy::DummyIO;
use crate::io::{IOBundle, temperatures};
use crate::io::gpio::sysfs_gpio::SysFsGPIO;
use crate::io::temperatures::dummy::ModifyState::SetTemp;
use crate::io::wiser::dummy::ModifyState;

mod io;
mod config;
mod brain;

const CONFIG_FILE: &str = "follow_heating.toml";
// TODO: LOOK INTO HOW HEAT CIRCULATION PUMP COULD HAVE BEEN LEFT ON AFTER SUPPOSED GRACEFUL SHUTDOWN.
fn main() {
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

    let brain = brain::python_like::PythonBrain::new();

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

fn make_gpio() -> (SysFsGPIO, Sender<PinUpdate>, Receiver<PinUpdate>) {
    let (tx, rx) = tokio::sync::mpsc::channel(5);

    let gpio = make_gpio_using(tx.clone());
    //let gpio = io::gpio::dummy::Dummy::new();
    return (gpio, tx, rx);
}

fn make_gpio_using(sender: Sender<PinUpdate>) -> SysFsGPIO {
    let mut gpio = io::gpio::sysfs_gpio::SysFsGPIO::new(sender);
    gpio.setup(5, &GPIOMode::Output);
    gpio.setup(HEAT_PUMP_RELAY, &GPIOMode::Output);
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
    println!("Current state: {:?}", gpio.get_pin(HEAT_PUMP_RELAY));

    loop {
        println!("Turning off");
        let before = Instant::now();
        gpio.set_pin(HEAT_PUMP_RELAY, &GPIOState::HIGH).unwrap();
        sleep(delay);
        gpio.set_pin(HEAT_PUMP_RELAY, &GPIOState::LOW).unwrap();
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

    let brain = brain::python_like::PythonBrain::new();

    let rt = Builder::new_multi_thread()
        .worker_threads(1)
        .enable_time()
        .enable_io()
        .build()
        .expect("Expected to be able to make runtime");

    //rt.spawn(io::gpio::update_db_with_gpio::run(pool.clone(), receiver));

    //sender.try_send(PinUpdate::new(1, GPIOState::LOW)).unwrap();

    rt.spawn(async move {
        tokio::time::sleep(Duration::from_secs(1)).await;
        temp_handle.send(SetTemp(Sensor::TKBT, 30.0)).unwrap();
        println!("Turning on fake wiser heating");
        tokio::time::sleep(Duration::from_secs(1)).await;
        wiser_handle.send(ModifyState::SetHeatingOffTime(Utc::now() + chrono::Duration::seconds(1000))).unwrap();
        tokio::time::sleep(Duration::from_secs(15)).await;
        println!("Setting TKBT to above the turn off temp.");
        temp_handle.send(SetTemp(Sensor::TKBT, 50.0)).unwrap();
        tokio::time::sleep(Duration::from_secs(60 * 8 + 30)).await;
        println!("Now turning back down.");
        temp_handle.send(SetTemp(Sensor::TKBT, 32.0)).unwrap();
        tokio::time::sleep(Duration::from_secs(300)).await
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
        G: GPIOManager + Send + 'static,
        W: WiserManager,
        F: FnOnce() -> G {

    let x = rt.block_on(io_bundle.wiser().get_wiser_hub().get_data());
    println!("Result {:?}", x);

    let should_exit = Arc::new(AtomicBool::new(false));

    {
        let should_exit = should_exit.clone();
        ctrlc::set_handler(move || {
            println!("Received termination signal."); // TODO: Handle SIGUSR signal for restarting?
            should_exit.store(true, Ordering::Relaxed);
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
        if should_exit.load(Ordering::Relaxed) {
            println!("Stopping safely...");
            shutdown_using_backup(rt, io_bundle, backup_gpio_supplier);
            // TODO: Check for important stuff going on.
            println!("Stopped safely.");
            return;
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
        sleep(Duration::from_secs(10));
        //futures::executor::block_on(interval.tick());
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
    where G: GPIOManager {
    rt.shutdown_background();
    let result = gpio.set_pin(brain::python_like::HEAT_PUMP_RELAY, &GPIOState::HIGH);
    if result.is_err() {
        println!("FAILED TO SHUTDOWN HEAT PUMP: {:?}", result.unwrap_err());
    }
    let result = gpio.set_pin(brain::python_like::HEAT_CIRCULATION_PUMP, &GPIOState::HIGH);
    if result.is_err() {
        println!("FAILED TO SHUTDOWN HEAT CIRCULATION PUMP: {:?}", result.unwrap_err());
    }
}
