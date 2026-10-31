pub mod app;
pub mod auth;
pub mod commands;
pub mod config;
pub mod connectors;
pub mod db;
pub mod embed;
pub mod error;
pub mod extract;
pub mod index;
pub mod indexer;
pub mod reveal;
pub mod search;
pub mod sources;
pub mod sync;
pub mod types;

use tauri::Manager; // required for app.path() and app.manage()

use crate::config::AppPaths;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let _ = dotenvy::from_filename("../.env").or_else(|_| dotenvy::dotenv());
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
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

            let conn = db::open(&paths.db_file())?;
            let search_index = index::SearchIndex::open(paths.index_dir())?;
            tracing::info!(db = %paths.db_file().display(), "database ready");

            // Never index our own database, index and logs, even if the user
            // adds a folder that contains them.
            let sync =
                sync::SyncService::start(&paths.db_file(), vec![paths.root().to_path_buf()])?;
            for source in db::list_sources(&conn)? {
                let patterns = db::list_exclusion_patterns(&conn, source.id)?;
                if let Err(e) = sync.start_source(&source, &patterns) {
                    tracing::error!(id = source.id, "could not start source: {e}");
                }
            }

            app.manage(app::AppState::new(paths, conn, sync, search_index, guard));

            #[cfg(desktop)]
            register_hotkey(app)?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::search,
            commands::add_source,
            commands::remove_source,
            commands::list_sources,
            commands::get_index_status,
            commands::start_oauth_login,
            //add command
            commands::cancel_oauth_login,
            //
            commands::open_location,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

/// Ctrl+Shift+Space shows or hides the window from anywhere.
/// Ctrl+Space is avoided because input methods use it, and Alt+Space opens
/// the window menu on Windows.
#[cfg(desktop)]
fn register_hotkey(app: &mut tauri::App) -> tauri::Result<()> {
    use tauri_plugin_global_shortcut::{
        Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState,
    };

    let toggle = Shortcut::new(Some(Modifiers::CONTROL | Modifiers::SHIFT), Code::Space);
    app.handle().plugin(
        tauri_plugin_global_shortcut::Builder::new()
            .with_handler(move |app, shortcut, event| {
                if shortcut == &toggle && event.state() == ShortcutState::Pressed {
                    app::toggle_main_window(app);
                }
            })
            .build(),
    )?;
    // Another app may own the shortcut, and Wayland blocks global shortcuts.
    // The app still works without it, so log instead of failing startup.
    if let Err(e) = app.global_shortcut().register(toggle) {
        tracing::warn!("global hotkey unavailable: {e}");
    }
    Ok(())
}
