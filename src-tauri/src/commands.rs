//! Tauri IPC command handlers invoked from the React frontend.
//!
//! Handlers stay thin: they unpack arguments and delegate to plain functions
//! in the other modules, so the real logic can be tested without a window.

use tauri::State;

use crate::app::AppState;
use crate::db;
use crate::error::Result;
use crate::types::{AddSourceRequest, IndexStatus, SearchRequest, SearchResponse, Source};

#[tauri::command(async)]
pub fn search(request: SearchRequest) -> Result<SearchResponse> {
    crate::search::search(&request)
}

#[tauri::command(async)]
pub fn add_source(state: State<'_, AppState>, request: AddSourceRequest) -> Result<Source> {
    let source = state.with_db(|conn| db::insert_source(conn, &request))?;
    tracing::info!(id = source.id, kind = request.kind.as_str(), "source added");
    Ok(source)
}

#[tauri::command(async)]
pub fn list_sources(state: State<'_, AppState>) -> Result<Vec<Source>> {
    state.with_db(db::list_sources)
}

#[tauri::command(async)]
pub fn get_index_status() -> Result<IndexStatus> {
    Ok(crate::sync::index_status())
}
