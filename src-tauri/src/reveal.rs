//! Opens the folder that holds a search result.
//!
//! Local and Obsidian files are shown in the system file manager with the
//! file selected. Cloud files open the containing folder's web page, except
//! Notion pages, which have no folders and open directly.

use std::path::Path;

use rusqlite::OptionalExtension;

use crate::app::AppState;
use crate::error::{AppError, Result};

/// Where a document's containing folder can be shown.
#[derive(Debug, PartialEq, Eq)]
enum Target {
    LocalFile(String),
    GoogleDrive(String),
    OneDrive(String),
    Notion(String),
}

/// Reads a document's `external_id` prefix to decide how to show it.
fn target_for(external_id: &str) -> Target {
    if let Some(id) = external_id.strip_prefix("gdrive:") {
        Target::GoogleDrive(id.into())
    } else if let Some(id) = external_id.strip_prefix("onedrive:") {
        Target::OneDrive(id.into())
    } else if let Some(id) = external_id.strip_prefix("notion:") {
        Target::Notion(id.into())
    } else {
        Target::LocalFile(external_id.into())
    }
}

fn account_id(config: &serde_json::Value) -> String {
    config
        .get("account_id")
        .and_then(|v| v.as_str())
        .unwrap_or("default")
        .to_string()
}

fn open_url(url: &str) -> Result<()> {
    tauri_plugin_opener::open_url(url, None::<&str>)
        .map_err(|e| AppError::Other(format!("Could not open {url}: {e}")))
}

/// Opens the folder containing document `document_id`.
pub async fn open_location(state: &AppState, document_id: i64) -> Result<()> {
    let (external_id, source_id): (String, i64) = state
        .with_db(|conn| {
            Ok(conn
                .query_row(
                    "SELECT external_id, source_id FROM documents WHERE id = ?1 AND deleted = 0",
                    [document_id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?)
        })?
        .ok_or_else(|| AppError::NotFound(format!("document {document_id}")))?;

    match target_for(&external_id) {
        Target::LocalFile(path) => {
            let path = Path::new(&path);
            if !path.exists() {
                return Err(AppError::NotFound(format!("file {}", path.display())));
            }
            tauri_plugin_opener::reveal_item_in_dir(path).or_else(|e| {
                // No file manager to select the file in: open its folder instead.
                tracing::warn!("Could not reveal {}: {e}; opening folder", path.display());
                let folder = path.parent().unwrap_or(path);
                tauri_plugin_opener::open_path(folder, None::<&str>).map_err(|e| {
                    AppError::Other(format!("Could not open {}: {e}", folder.display()))
                })
            })
        }
        Target::GoogleDrive(file_id) => {
            let source = state.with_db(|conn| crate::db::get_source(conn, source_id))?;
            let connector =
                crate::connectors::GoogleDriveConnector::new(account_id(&source.config));
            open_url(&connector.folder_url(&file_id).await?)
        }
        Target::OneDrive(item_id) => {
            let source = state.with_db(|conn| crate::db::get_source(conn, source_id))?;
            let connector = crate::connectors::OneDriveConnector::new(account_id(&source.config));
            open_url(&connector.folder_url(&item_id).await?)
        }
        Target::Notion(page_id) => open_url(&format!(
            "https://www.notion.so/{}",
            page_id.replace('-', "")
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn external_id_prefix_picks_the_target() {
        assert_eq!(target_for("gdrive:abc"), Target::GoogleDrive("abc".into()));
        assert_eq!(target_for("onedrive:x1"), Target::OneDrive("x1".into()));
        assert_eq!(target_for("notion:p-1"), Target::Notion("p-1".into()));
        assert_eq!(
            target_for("/home/me/notes/a.md"),
            Target::LocalFile("/home/me/notes/a.md".into())
        );
    }
}
