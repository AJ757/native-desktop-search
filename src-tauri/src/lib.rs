pub mod app;
pub mod commands;
pub mod config;
pub mod db;
pub mod embed;
pub mod error;
pub mod extract;
pub mod index;
pub mod search;
pub mod sources;
pub mod sync;
pub mod types;

use tauri::Manager; // required for app.path() and app.manage()

use crate::config::AppPaths;

// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let root = app.path().app_local_data_dir()?;
            let paths = AppPaths::new(root);
            paths.ensure_dirs()?;

            let guard = app::init_logging(&paths.logs_dir())?;
            app::install_panic_hook();

            tracing::info!(
                version = env!("CARGO_PKG_VERSION"),
                data_dir = %paths.root().display(),
                "Native Desktop Search starting"
            );

            app.manage(app::LogGuard(guard)); // keeps logging alive
            app.manage(paths);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![greet])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
