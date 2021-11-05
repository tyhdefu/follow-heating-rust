use std::fs;
use std::ops::Add;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::sleep;
use std::time::{Duration, Instant};
use sqlx::MySqlPool;
use tokio::runtime::{Builder, Runtime};
use crate::config::{Config, DatabaseConfig};
use crate::io::gpio::dummy::Dummy;
use crate::io::gpio::{GPIOManager, GPIOMode};
use crate::io::temperatures::database::DBTemperatureManager;
use crate::io::temperatures::TemperatureManager;
use crate::io::wiser::WiserManager;
use io::wiser;
use crate::brain::Brain;
use crate::io::dummy::DummyIO;
use crate::io::{IOBundle, temperatures};
use crate::io::wiser::dummy::ModifyState;

mod io;
mod config;
mod brain;

const CONFIG_FILE: &str = "follow_heating.toml";

fn main() {
    /*let config = fs::read_to_string(CONFIG_FILE)
        .expect("Unable to read test config file. Is it missing?");
    let config: Config = toml::from_str(&*config)
        .expect("Error reading test config file");

    let db_url = make_db_url(config.get_database());
    let pool = futures::executor::block_on(MySqlPool::connect(&db_url)).expect("to connect");
    let mut temps = DBTemperatureManager::new(pool);
    futures::executor::block_on(temps.retrieve_sensors());
    let cur_temps = futures::executor::block_on(temps.retrieve_temperatures()).expect("Failed to retrieve temperatures");
    println!("{:?}", cur_temps);*/

    /*let (temp_manager, temp_handle) = temperatures::dummy::Dummy::create();

    let gpios = gpio::dummy::Dummy::new();
    let (wiser, wiser_handle) = wiser::dummy::Dummy::create();

    let mut io_bundle = IOBundle::new(temp_manager, gpios, wiser);

    let brain = brain::dummy::Dummy::new();

    let rt = Builder::new_multi_thread()
        .worker_threads(1)
        .enable_time()
        .build()
        .expect("Expected to be able to make runtime");

    rt.spawn(async move {
        main_loop(brain, io_bundle);
    });*/

    let mut gpio = io::gpio::sysfs_gpio::SysFsGPIO::new();
    gpio.setup(1, &GPIOMode::Input);
    let value = gpio.get_pin(1).expect("Expected to read thing.");
    println!("{:?}", value);
}

fn make_db_url(db_config: &DatabaseConfig) -> String {
    format!("mysql://{}:{}@localhost:{}/{}", db_config.get_user(), db_config.get_password(), db_config.get_port(), db_config.get_database())
}

fn main_loop<B, T, G, W>(mut brain: B, mut io_bundle: IOBundle<T, G, W>)
    where
        B: Brain,
        T: TemperatureManager,
        G: GPIOManager,
        W: WiserManager, {

    let rt = Builder::new_multi_thread()
        .worker_threads(1)
        .enable_time()
        .build()
        .expect("Expected to be able to make runtime");

    let should_exit = Arc::new(AtomicBool::new(false));

    {
        let should_exit = should_exit.clone();
        ctrlc::set_handler(move || {
            println!("Received termination signal.");
            should_exit.store(true, Ordering::Relaxed);
        }).expect("Failed to attach kill handler.");
    }

    loop {
        if should_exit.load(Ordering::Relaxed) {
            println!("Stopping safely...");
            rt.shutdown_background(); // TODO: Check for important stuff going on.
            println!("Stopped safely.");
            return;
        }

        brain.run(&rt, &mut io_bundle);

        sleep(Duration::from_secs(1));
    }
}