use std::fs;

use time::UtcOffset;
use tracing::{Level, Subscriber};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{reload::Handle, EnvFilter};

pub fn init_logging() -> Result<LoggingHandle<EnvFilter, impl Subscriber>, String> {
    init_tracing_logger()
}

pub enum ReloadLogLevelError {
    InvalidFilter(String),
    ReloadFailed(tracing_subscriber::reload::Error),
}

pub fn reload_log_level(
    logging_handle: &LoggingHandle<EnvFilter, impl Subscriber>,
) -> Result<String, ReloadLogLevelError> {
    let new_filter = read_env_filter().map_err(|e| ReloadLogLevelError::InvalidFilter(e))?;

    let filter_string = format!("{}", new_filter);

    logging_handle
        .handle
        .reload(new_filter)
        .map_err(|e| ReloadLogLevelError::ReloadFailed(e))?;

    Ok(filter_string)
}

fn init_tracing_logger() -> Result<LoggingHandle<EnvFilter, impl Subscriber>, String> {
    let timer = tracing_subscriber::fmt::time::OffsetTime::new(
        UtcOffset::current_local_offset().unwrap_or_else(|err| {
            eprintln!("Failed to get timezone: {}", err);
            UtcOffset::UTC
        }),
        time::macros::format_description!(
            "[year]-[month]-[day] [hour]:[minute]:[second] +[offset_hour]"
        ),
    );
    let (non_blocking, guard) = tracing_appender::non_blocking(std::io::stdout());

    let env_filter = read_env_filter().unwrap_or_else(|err| {
        eprintln!(
            "Failed to read env filter, using environment variable or default: {}",
            err
        );
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

fn read_env_filter() -> Result<EnvFilter, String> {
    let s = fs::read_to_string("logging.env")
        .map_err(|err| format!("Failed to read file logging.env file: {}", err))?;
    let first_line = s.lines().next().expect("Should have atleast one line");
    Ok(EnvFilter::builder()
        .with_default_directive(tracing::Level::DEBUG.into())
        .parse(first_line)
        .map_err(|err| format!("Failed to parse env filter: {}", err))?)
}

pub struct LoggingHandle<L, S> {
    non_blocking_guard: WorkerGuard,
    handle: Handle<L, S>,
}
