use std::fs;
use std::net::Ipv4Addr;
use std::ops::DerefMut;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::sleep;
use std::time::{Duration};
use chrono::Utc;
use sqlx::MySqlPool;
use tokio::runtime::{Builder, Runtime};
use crate::config::{Config, DatabaseConfig};
use crate::io::gpio::{GPIOManager, GPIOState};
use crate::io::temperatures::database::DBTemperatureManager;
use crate::io::temperatures::{Sensor, TemperatureManager};
use crate::io::wiser::WiserManager;
use io::wiser;
use crate::brain::Brain;
use crate::io::dummy::DummyIO;
use crate::io::{IOBundle, temperatures};
use crate::io::temperatures::dummy::ModifyState::SetTemp;
use crate::io::wiser::dummy::ModifyState;

mod io;
mod config;
mod brain;

const CONFIG_FILE: &str = "follow_heating.toml";

fn main() {
    let config = fs::read_to_string(CONFIG_FILE)
        .expect("Unable to read test config file. Is it missing?");
    let config: Config = toml::from_str(&*config)
        .expect("Error reading test config file");

    let db_url = make_db_url(config.get_database());
    let pool = futures::executor::block_on(MySqlPool::connect(&db_url)).expect("to connect");
    let mut temps = DBTemperatureManager::new(pool.clone());
    futures::executor::block_on(temps.retrieve_sensors()).unwrap();
    let cur_temps = futures::executor::block_on(temps.retrieve_temperatures()).expect("Failed to retrieve temperatures");
    println!("{:?}", cur_temps);

    let gpios = io::gpio::dummy::Dummy::new();
    let secret = "*SECRET*";
    let wiser = wiser::dbhub::DBAndHub::new(pool, Ipv4Addr::new(192, 168, 0, 28).into(), secret.to_owned());

    let backup_gpio_supplier = || io::gpio::dummy::Dummy::new();
    let io_bundle = IOBundle::new(temps, gpios, wiser);

    let brain = brain::python_like::PythonBrain::new();

    let rt = Builder::new_multi_thread()
        .worker_threads(1)
        .enable_time()
        .build()
        .expect("Expected to be able to make runtime");

    rt.spawn(async move {
        main_loop(brain, io_bundle, backup_gpio_supplier);
    });
}

fn simulate() {
    let gpios = io::gpio::dummy::Dummy::new();
    let (wiser, wiser_handle) = wiser::dummy::Dummy::create();
    let (temp_manager, temp_handle) = temperatures::dummy::Dummy::create();

    let backup_gpio_supplier = || io::gpio::dummy::Dummy::new();

    let io_bundle = IOBundle::new(temp_manager, gpios, wiser);

    let brain = brain::python_like::PythonBrain::new();

    let rt = Builder::new_multi_thread()
        .worker_threads(1)
        .enable_time()
        .build()
        .expect("Expected to be able to make runtime");

    rt.spawn(async move {
        main_loop(brain, io_bundle, backup_gpio_supplier);
    });

    temp_handle.send(SetTemp(Sensor::TKBT, 30.0)).unwrap();
    println!("Turning on fake wiser heating");
    sleep(Duration::from_secs(3));
    wiser_handle.send(ModifyState::SetHeatingOffTime(Utc::now() + chrono::Duration::seconds(1000))).unwrap();
    sleep(Duration::from_secs(10));
    println!("Setting TKBT to above the turn off temp.");
    temp_handle.send(SetTemp(Sensor::TKBT, 50.0)).unwrap();
    sleep(Duration::from_secs(5 * 60));
    println!("Now turning back down.");
    temp_handle.send(SetTemp(Sensor::TKBT, 32.0)).unwrap();
    sleep(Duration::from_secs(30));
    println!("Turning off heating.");
    wiser_handle.send(ModifyState::TurnOffHeating).unwrap();
}

fn make_db_url(db_config: &DatabaseConfig) -> String {
    format!("mysql://{}:{}@localhost:{}/{}", db_config.get_user(), db_config.get_password(), db_config.get_port(), db_config.get_database())
}

fn main_loop<B, T, G, W, F>(mut brain: B, mut io_bundle: IOBundle<T, G, W>, backup_gpio_supplier: F)
    where
        B: Brain,
        T: TemperatureManager,
        G: GPIOManager + Send + 'static,
        W: WiserManager,
        F: FnOnce() -> G {
    let rt = Builder::new_multi_thread()
        .worker_threads(1)
        .enable_time()
        .build()
        .expect("Expected to be able to make runtime");

    let should_exit = Arc::new(AtomicBool::new(false));

    {
        let should_exit = should_exit.clone();
        ctrlc::set_handler(move || {
            println!("Received termination signal."); // TODO: Handle SIGUSR signal for restarting?
            should_exit.store(true, Ordering::Relaxed);
        }).expect("Failed to attach kill handler.");
    }

    let mut i = 0;
    loop {
        i += 1;
        if i % 60 == 0 {
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

        sleep(Duration::from_secs(1));
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