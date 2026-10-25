//! Application setup and shared state.
//!
//! Builds the `AppState` (database handle, data paths, logging guard) that
//! Tauri manages, and runs startup steps such as creating directories,
//! initialising logging and migrating the database.

use std::path::Path;

use crate::config::AppPaths;
use rusqlite::Connection;
use std::sync::Mutex;

use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::{Builder, Rotation};
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

use crate::error::{AppError, Result};

pub struct AppState {
    pub paths: AppPaths,
    db: Mutex<Connection>,
    _log_guard: WorkerGuard, // keeps file logging alive; never read
}

impl AppState {
    pub fn new(paths: AppPaths, conn: Connection, log_guard: WorkerGuard) -> Self {
        Self {
            paths,
            db: Mutex::new(conn),
            _log_guard: log_guard,
        }
    }

    /// Run a closure with the database connection locked.
    pub fn with_db<T>(&self, f: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
        let conn = self
            .db
            .lock()
            .map_err(|_| AppError::Other("database lock poisoned".into()))?;
        f(&conn)
    }
}

pub fn init_logging(logs_dir: &Path) -> Result<WorkerGuard> {
    let appender = Builder::new()
        .rotation(Rotation::DAILY)
        .filename_prefix("native-desktop-search")
        .filename_suffix("log")
        .max_log_files(14) // delete older files so logs can't grow forever
        .build(logs_dir)
        .map_err(|e| AppError::Other(format!("could not create log file: {e}")))?;
    let (file_writer, guard) = tracing_appender::non_blocking(appender);

    // RUST_LOG wins if set; otherwise use these defaults.
    let default_filter = if cfg!(debug_assertions) {
        "info,native_desktop_search_lib=debug"
    } else {
        "info"
    };
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_filter));

    // Console output in dev builds only.
    let console = cfg!(debug_assertions).then(|| fmt::layer().with_writer(std::io::stderr));

    tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer().with_writer(file_writer).with_ansi(false))
        .with(console)
        .try_init()
        .map_err(|e| AppError::Other(format!("logging already initialised: {e}")))?;

    Ok(guard)
}

/// Log panics through tracing (so they reach the log file), then run the default hook.
pub fn install_panic_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let location = info
            .location()
            .map(|l| format!("{}:{}", l.file(), l.line()))
            .unwrap_or_else(|| "unknown".into());
        let payload = info.payload();
        let message = payload
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| payload.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "non-string panic payload".into());
        tracing::error!(%location, "panic: {message}");
        default_hook(info);
    }));
}
