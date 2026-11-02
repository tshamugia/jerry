//! Jerry's log: tracing to `%LOCALAPPDATA%\Jerry\logs`, one file per day, the last
//! [`LOG_FILES`] days kept. Dev builds also log to the console.
//!
//! A background thread writes the file, so logging never does file IO on the calling thread.
//! Panics are logged too, from a panic hook: thread, location and backtrace.

use std::backtrace::Backtrace;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::{Mutex, PoisonError};

use jerry_core::JerryError;
use tracing::level_filters::LevelFilter;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::filter::Targets;
use tracing_subscriber::prelude::*;
use tracing_subscriber::{Layer, Registry, fmt};

/// Log files kept: one per day, so 14 days of logs.
pub const LOG_FILES: usize = 14;

/// Overrides the default filter, for example `JERRY_LOG=debug` or `JERRY_LOG=jerry_core=trace,info`.
const FILTER_VAR: &str = "JERRY_LOG";

/// Keeps the file writer's thread alive; dropping it writes out what is still buffered.
static FILE_WRITER: Mutex<Option<WorkerGuard>> = Mutex::new(None);

/// Where the log files go: `%LOCALAPPDATA%\Jerry\logs`.
pub fn log_dir() -> Result<PathBuf, JerryError> {
    Ok(jerry_win::folders::local_app_data()?
        .join("Jerry")
        .join("logs"))
}

/// Starts logging and logs every panic from then on. Call once, before anything else: Jerry
/// runs without a log file if it can't write one, and says so on the console (dev builds).
pub fn init() {
    init_at(log_dir());
}

/// Writes out the lines still buffered. Call when Jerry exits: tao ends the process without
/// running destructors.
pub fn flush() {
    let writer = FILE_WRITER
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .take();
    drop(writer);
}

fn init_at(dir: Result<PathBuf, JerryError>) {
    let (file, file_dir) = match dir.and_then(|dir| file_layer(&dir).map(|layer| (layer, dir))) {
        Ok((layer, dir)) => (Some(layer), Ok(dir)),
        Err(error) => (None, Err(error)),
    };
    let console = cfg!(debug_assertions).then(|| fmt::layer().with_writer(std::io::stderr));
    let installed = tracing_subscriber::registry()
        .with(file)
        .with(console)
        .with(filter())
        .try_init();
    if installed.is_err() {
        return; // a subscriber is already set (a second call)
    }
    std::panic::set_hook(Box::new(log_panic));

    match file_dir {
        Ok(dir) => tracing::info!(dir = %dir.display(), "logging"),
        Err(error) => {
            tracing::error!(%error, "can't write the log file; logging to the console only");
        }
    }
}

fn file_layer(dir: &Path) -> Result<impl Layer<Registry> + Send + Sync + use<>, JerryError> {
    std::fs::create_dir_all(dir).map_err(|error| JerryError::from(error).at(dir))?;
    let appender = RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix("jerry")
        .filename_suffix("log")
        .max_log_files(LOG_FILES)
        .build(dir)
        .map_err(|error| JerryError::from(std::io::Error::other(error)).at(dir))?;
    let (writer, guard) = tracing_appender::non_blocking(appender);
    *FILE_WRITER.lock().unwrap_or_else(PoisonError::into_inner) = Some(guard);
    Ok(fmt::layer()
        .with_writer(writer)
        .with_ansi(false)
        .with_thread_names(true))
}

/// Jerry's own crates log at info (debug in dev builds), everything else (Tauri, wry) at warn.
/// `JERRY_LOG` replaces this.
fn filter() -> Targets {
    if let Some(custom) = std::env::var(FILTER_VAR)
        .ok()
        .and_then(|value| Targets::from_str(&value).ok())
    {
        return custom;
    }
    let ours = if cfg!(debug_assertions) {
        LevelFilter::DEBUG
    } else {
        LevelFilter::INFO
    };
    Targets::new()
        .with_default(LevelFilter::WARN)
        .with_target("jerry_app", ours)
        .with_target("jerry_core", ours)
        .with_target("jerry_win", ours)
}

fn log_panic(info: &std::panic::PanicHookInfo<'_>) {
    let thread = std::thread::current();
    let thread = thread.name().unwrap_or("unnamed");
    let message = info.payload_as_str().unwrap_or("(no message)");
    let location = info.location().map(ToString::to_string).unwrap_or_default();
    let backtrace = Backtrace::force_capture();
    tracing::error!("panic in thread '{thread}' at {location}: {message}\n{backtrace}");
}

#[cfg(test)]
mod tests {
    use jerry_core::ErrorCode;
    use jerry_core::job::run_job;

    use super::*;

    fn log_files(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .expect("log dir")
            .map(|entry| {
                entry
                    .expect("entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        names.sort();
        names
    }

    // One test only: the subscriber and the panic hook are process-wide.
    #[test]
    fn a_worker_panic_is_logged_and_old_logs_are_pruned() {
        let dir = tempfile::tempdir().expect("temp dir");
        for day in 1..=20 {
            let old = dir.path().join(format!("jerry.2026-10-{day:02}.log"));
            std::fs::write(old, "old\n").expect("old log");
        }

        init_at(Ok(dir.path().to_path_buf()));
        let worker = std::thread::Builder::new()
            .name("jerry-test-worker".into())
            .spawn(|| run_job::<()>("test job", || panic!("deliberate panic in a job")))
            .expect("spawn");
        let failed = worker.join().expect("the worker thread survives the panic");
        assert_eq!(failed.map_err(|error| error.code), Err(ErrorCode::Internal));
        tracing::info!("still running after the panic");
        flush();

        let names = log_files(dir.path());
        assert_eq!(names.len(), LOG_FILES, "{names:?}");
        assert_eq!(
            names.first().map(String::as_str),
            Some("jerry.2026-10-08.log")
        );
        let today = names.last().expect("today's log");
        let log = std::fs::read_to_string(dir.path().join(today)).expect("today's log");
        assert!(
            log.contains("panic in thread 'jerry-test-worker'"),
            "the panic hook logged:\n{log}"
        );
        assert!(log.contains("deliberate panic in a job"), "{log}");
        assert!(log.contains(r#"job="test job""#), "run_job logged:\n{log}");
        assert!(log.contains("still running after the panic"), "{log}");
    }
}
