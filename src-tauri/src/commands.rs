//! Tauri IPC command handlers invoked from the React frontend.
//!
//! Handlers stay thin: they unpack arguments and delegate to plain functions
//! in the other modules, so the real logic can be tested without a window.

use tauri::State;

use crate::app::AppState;
use crate::db;
use crate::error::{AppError, Result};
use crate::sources;
use crate::types::{AddSourceRequest, IndexStatus, SearchRequest, SearchResponse, Source};

#[tauri::command(async)]
pub fn search(request: SearchRequest) -> Result<SearchResponse> {
    crate::search::search(&request)
}

/// Adds a source and starts syncing it. Folder sources must point at an
/// existing folder that is not already added; the stored path is absolute.
#[tauri::command(async)]
pub fn add_source(state: State<'_, AppState>, request: AddSourceRequest) -> Result<Source> {
    let request = if sources::is_folder_kind(request.kind) {
        let root = sources::folder_root_from_config(&request.config)?;
        let existing = state.with_db(db::list_sources)?;
        let duplicate = existing.iter().any(|s| {
            sources::is_folder_kind(s.kind)
                && sources::folder_root_from_config(&s.config).is_ok_and(|r| r == root)
        });
        if duplicate {
            return Err(AppError::Config(format!(
                "folder is already added: {}",
                root.display()
            )));
        }
        let mut config = request.config;
        config["path"] = serde_json::Value::String(sources::path_key(&root));
        AddSourceRequest {
            kind: request.kind,
            config,
        }
    } else {
        request
    };

    let (source, patterns) = state.with_db(|conn| {
        let source = db::insert_source(conn, &request)?;
        let patterns = db::list_exclusion_patterns(conn, source.id)?;
        Ok((source, patterns))
    })?;
    tracing::info!(id = source.id, kind = request.kind.as_str(), "source added");
    state.sync.start_source(&source, &patterns)?;
    Ok(source)
}

/// Stops syncing a source and deletes it with all of its documents.
#[tauri::command(async)]
pub fn remove_source(state: State<'_, AppState>, id: i64) -> Result<()> {
    state.sync.stop_source(id);
    state.with_db(|conn| db::delete_source(conn, id))?;
    tracing::info!(id, "source removed");
    Ok(())
}

#[tauri::command(async)]
pub fn list_sources(state: State<'_, AppState>) -> Result<Vec<Source>> {
    state.with_db(db::list_sources)
}

#[tauri::command(async)]
pub fn get_index_status(state: State<'_, AppState>) -> Result<IndexStatus> {
    let total = state.with_db(db::count_documents)?;
    Ok(state.sync.status(total))
}
