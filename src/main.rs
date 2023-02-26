use std::borrow::BorrowMut;
use std::{fs, panic};
use std::fmt::Debug;
use std::ops::DerefMut;
use std::time::Duration;
use log::{debug, error, info, trace};
use sqlx::MySqlPool;
use time::UtcOffset;
use tokio::runtime::{Builder, Runtime};
use tokio::signal::unix::SignalKind;
use tokio::sync::mpsc::{Receiver, Sender};
use tracing::{Level, Subscriber};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_log::LogTracer;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::reload::Handle;
use brain::python_like;
use brain::python_like::config::PythonBrainConfig;
use crate::config::{Config, DatabaseConfig};
use crate::io::devices::DevicesFromFile;
use crate::io::gpio::{GPIOError, GPIOManager, GPIOMode, GPIOState, PinUpdate};
use crate::io::temperatures::database::DBTemperatureManager;
use crate::io::temperatures::{Sensor, TemperatureManager};
use crate::io::wiser::WiserManager;
use io::wiser;
use crate::brain::{Brain, BrainFailure, CorrectiveActions};
use crate::io::IOBundle;
use crate::io::controls::heating_impl::GPIOHeatingControl;
use crate::io::controls::misc_impl::MiscGPIOControls;
use crate::io::gpio::sysfs_gpio::SysFsGPIO;
use crate::python_like::config::try_read_python_brain_config;
use crate::python_like::control::heating_control::HeatingControl;
use crate::python_like::control::misc_control::MiscControls;
use crate::time_util::mytime::{RealTimeProvider, TimeProvider};
use crate::wiser::hub::WiserHub;
use crate::brain::python_like::control::misc_control::ImmersionHeaterControl;

mod io;
mod config;
mod brain;
mod math;
mod time_util;
mod simulate;

const CONFIG_FILE: &str = "follow_heating.toml";

fn check_config() {
    let config = fs::read_to_string(CONFIG_FILE)
        .expect("Unable to read test config file. Is it missing?");
    let _config: Config = toml::from_str(&*config)
        .expect("Error reading test config file");

    try_read_python_brain_config().expect("Failed to read python brain config.");
}

fn read_env_filter() -> Result<EnvFilter, String> {
    let s = fs::read_to_string("logging.env")
        .map_err(|err| format!("Failed to read file logging.env file: {}", err))?;
    let first_line = s.lines().next().expect("Should have atleast one line");
    Ok(EnvFilter::builder()
        .with_default_directive(tracing::Level::DEBUG.into())
        .parse(first_line)
        .map_err(|err| format!("Failed to parse env filter: {}", err))?
    )

}

fn init_tracing_logger() -> Result<LoggingHandle<EnvFilter, impl Subscriber>, String> {
    let timer = tracing_subscriber::fmt::time::OffsetTime::new(
        UtcOffset::current_local_offset().unwrap_or_else(|err| {
            eprintln!("Failed to get timezone: {}", err);
            UtcOffset::UTC
        }),
        time::macros::format_description!("[year]-[month]-[day] [hour]:[minute]:[second] +[offset_hour]")
    );
    let (non_blocking, guard) = tracing_appender::non_blocking(std::io::stdout());

    let env_filter = read_env_filter()
        .unwrap_or_else(|err| {
            eprintln!("Failed to read env filter, using environment variable or default: {}", err);
            EnvFilter::builder()
                .with_default_directive(Level::DEBUG.into())
                .from_env_lossy()
        });

    println!("Env Filter: {}", env_filter);

    //let (layer, handle) = reload::Layer::new(env_filter);

    let builder = tracing_subscriber::fmt()
        .with_timer(timer)
        .with_writer(non_blocking)
        .with_env_filter(env_filter)
        .with_filter_reloading();

    //let layered = subscriber.with(env_filter);
    let handle = builder.reload_handle();

    tracing::subscriber::set_global_default(builder.finish())
        .map_err(|err| format!("failed to initialize logger: {}", err))?;
    Ok(LoggingHandle {
        non_blocking_guard: guard,
        handle,
    })
}

pub struct LoggingHandle<L, S> {
    non_blocking_guard: WorkerGuard,
    handle: Handle<L,S>,
}

fn main() {
    // Make tokio convert log::info! etc. into tracing "events"
    LogTracer::init().expect("Should be able to make tokio subscribers listen to the log crate!");

    let logging_handle = init_tracing_logger().expect("Failed to initialize logger");

    info!("Hopefully this is logging!");

    let args: Vec<String> = std::env::args().collect();
    if args.len() == 1 && args[0] == "check-config" {
        if args[0] == "check-config" {
            check_config();
            info!("Config OK!");
        }
        error!("Unrecognized argument: {}, run with no args to run normally.", args.len());
        return;
    }

    info!("Preparing...");

    let config = fs::read_to_string(CONFIG_FILE)
        .expect("Unable to read test config file. Is it missing?");
    let config: Config = toml::from_str(&*config)
        .expect("Error reading test config file");

    let default_hook = panic::take_hook();
    panic::set_hook(Box::new(move |panic| {
        error!("PANICKED: {:?}: Shutting down", panic);
        let (send, _recv) = tokio::sync::mpsc::channel(1);
        match make_controls(send) {
            Ok((mut heating_controls, mut misc_controls)) => {
                shutdown_heating(&mut heating_controls);
                shutdown_misc(&mut misc_controls);
            },
            Err(e) => {
                error!("Failed to get access to controls, anything could be on/off: {}", e);
            }
        }
        error!("Warning: Unlikely to have recorded state correctly in database.");
        default_hook(panic);
    }));

    if cfg!(debug_assertions) {
        simulate::simulate(logging_handle);
        panic!("Testing.");
    }

    // Read brain config.
    let python_brain_config = read_python_brain_config();

    info!(target: "config", "python brain config {:?}", &python_brain_config);

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

    main_loop(brain, io_bundle, rt, backup_supplier, RealTimeProvider::default(), logging_handle);
}

fn read_python_brain_config() -> PythonBrainConfig {
    match python_like::config::try_read_python_brain_config() {
        None => {
            error!("Using default config as couldn't read python brain config");
            PythonBrainConfig::default()
        }
        Some(config) => config
    }
}

fn make_io_bundle(config: Config, pool: MySqlPool) -> Result<(IOBundle, Sender<PinUpdate>, Receiver<PinUpdate>), Box<BrainFailure>> {
    let mut temps = DBTemperatureManager::new(pool.clone());
    futures::executor::block_on(temps.retrieve_sensors()).unwrap();
    let cur_temps = futures::executor::block_on(temps.retrieve_temperatures()).expect("Failed to retrieve temperatures");
    info!("{:?}", cur_temps);

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

fn make_db_url(db_config: &DatabaseConfig) -> String {
    format!("mysql://{}:{}@localhost:{}/{}", db_config.get_user(), db_config.get_password(), db_config.get_port(), db_config.get_database())
}

fn main_loop<B, H, F>(mut brain: B, mut io_bundle: IOBundle, rt: Runtime, backup_supplier: F, time_provider: impl TimeProvider, logging_handle: LoggingHandle<EnvFilter, impl Subscriber>)
    where
        B: Brain,
        H: HeatingControl,
        F: FnOnce() -> H {

    let x = rt.block_on(io_bundle.wiser().get_wiser_hub().get_data());
    debug!("Result {:?}", x);


    let (signal_send, mut signal_recv) = tokio::sync::mpsc::channel(5);

    #[cfg(target_family = "unix")]
    {
        debug!("Subscribing to signals.");
        subscribe_signal(&rt, SignalKind::interrupt(), signal_send.clone(), Signal::Stop);
        subscribe_signal(&rt, SignalKind::terminate(), signal_send.clone(), Signal::Stop);
        subscribe_signal(&rt, SignalKind::user_defined1(), signal_send.clone(), Signal::Reload);
    }
    #[cfg(not(target_family = "unix"))]
    {
        let signal_send = signal_send.clone();
        ctrlc::set_handler(move || {
            info!("Received termination signal.");
            signal_send.blocking_send(Signal::Stop).unwrap();
        }).expect("Failed to attach kill handler.");
    }

    //let mut interval = tokio::time::interval(Duration::from_secs(2));
    //interval.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut i = 0;
    info!("Beginning main loop.");
    loop {
        i += 1;
        if i % 6 == 0 {
            info!("Still alive..")
        }

        let result = brain.run(&rt, &mut io_bundle, &time_provider);
        if let Err(err) = result {
            error!("Brain Failure: {}", err);
            // TODO: Handle corrective actions.
            error!("Shutting down.");
            let _ = panic::take_hook(); // Remove our custom panic hook.
            shutdown_using_backup(rt, io_bundle, backup_supplier);
            panic!("Had brain failure: see above.");
        }
        if let Some(signal) = rt.block_on(wait_or_get_signal(&mut signal_recv))  {
            info!("Received signal to {:?}", signal);
            match signal {
                Signal::Stop => {
                    info!("Stopping safely...");
                    shutdown_using_backup(rt, io_bundle, backup_supplier);
                    // TODO: Check for important stuff going on.
                    info!("Stopped safely.");
                    return;
                }
                Signal::Reload => {
                    info!("Reloading");
                    debug!("Reloading logging filter");
                    match read_env_filter() {
                        Ok(new_filter) => {
                            info!("New filter: {}", new_filter);
                            match logging_handle.handle.reload(new_filter) {
                                Ok(_) => info!("Applied to new logging filter successfully"),
                                Err(err) => error!("Failed to apply new logging filter: {}", err),
                            };
                        }
                        Err(err) => {
                            error!("Failed to read env filter: {}", err);
                        }
                    }
                    debug!("Reloading python brain config");
                    brain.reload_config();
                    info!("Reloading config complete")
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
    Reload,
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

    info!("Waiting for database inserts to be processed.");
    // TODO: Shutdown updater thread.
    rt.shutdown_timeout(Duration::from_millis(1000));
}

fn shutdown_misc(misc_controls: &mut dyn MiscControls) {
    if let Err(e) = misc_controls.try_set_immersion_heater(false) {
        error!("FAILED TO SHUTDOWN IMMERSION HEATER: {:?}. It may still be on", e);
    }
    if let Err(e) = misc_controls.try_set_wiser_power(true) {
        error!("FAILED TO TURN BACK ON WISER POWER: {:?}. It may be off.", e);
    }
}

fn shutdown_heating(heating_control: &mut dyn HeatingControl) {
    if let Err(e) = heating_control.try_set_heat_pump(false) {
        error!("FAILED TO SHUTDOWN HEAT PUMP: {:?}. It may still be on", e);
    }
    if let Err(e) = heating_control.try_set_heat_circulation_pump(false) {
        error!("FAILED TO SHUTDOWN HEAT CIRCULATION PUMP: {:?}. It may still be on", e);
    }
}

