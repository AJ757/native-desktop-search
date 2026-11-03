//! Background indexer.
//!
//! Keeps the Tantivy index and the stored embeddings in step with the
//! `documents` table. The sync service clears `indexed_at` whenever a document
//! is added or changes; this indexer picks those rows up, indexes each one
//! once, and drops deleted documents from the index. Searches never wait for it.

use std::collections::HashMap;
use std::path::Path;
use std::time::{Duration, Instant};

use rusqlite::{Connection, OptionalExtension};
use tauri::Manager;

use crate::app::AppState;
use crate::error::{AppError, Result};
use crate::extract::{ExtractedDocument, Extractor};
use crate::index::IndexedDocument;

/// How often the indexer looks for new, changed or removed documents.
const POLL_INTERVAL: Duration = Duration::from_secs(2);

/// Documents indexed per commit, so results show up while a large folder is
/// still being processed.
const BATCH_SIZE: usize = 25;

/// How long to wait before retrying a document whose text could not be
/// downloaded.
const RETRY_AFTER: Duration = Duration::from_secs(300);

struct DocumentRow {
    id: i64,
    source_id: i64,
    path: String,
    title: String,
    modified_at: Option<i64>,
    mime: Option<String>,
}

fn document_row(row: &rusqlite::Row) -> rusqlite::Result<DocumentRow> {
    Ok(DocumentRow {
        id: row.get(0)?,
        source_id: row.get(1)?,
        path: row.get(2)?,
        title: row.get(3)?,
        modified_at: row.get(4)?,
        mime: row.get(5)?,
    })
}

/// Text for a Google Drive document. Text saved by an earlier rebuild is
/// reused unless Drive reports a newer modification, so searches don't
/// download every file again. Files with no usable text are indexed by title.
fn google_drive_text(
    state: &AppState,
    document: &DocumentRow,
    file_id: &str,
) -> Result<ExtractedDocument> {
    let cached: Option<String> = state.with_db(|conn| {
        Ok(conn
            .query_row(
                "SELECT content FROM document_embeddings
                 WHERE document_id = ?1 AND updated_at >= COALESCE(?2, 0)",
                rusqlite::params![document.id, document.modified_at],
                |row| row.get(0),
            )
            .optional()?)
    })?;

    let text = match cached {
        Some(text) => text,
        None => {
            let source = state.with_db(|conn| crate::db::get_source(conn, document.source_id))?;
            let account_id = source
                .config
                .get("account_id")
                .and_then(|v| v.as_str())
                .unwrap_or("default")
                .to_string();
            let connector = crate::connectors::GoogleDriveConnector::new(account_id);
            let (file_id, title, mime) =
                (file_id, document.title.as_str(), document.mime.as_deref());
            block_on(async move { connector.download_text(file_id, title, mime).await })??
        }
    };

    Ok(ExtractedDocument {
        title: document.title.clone(),
        text: if text.trim().is_empty() {
            document.title.clone()
        } else {
            text
        },
    })
}

/// Text for a Notion page. Like Google Drive, text saved by an earlier
/// rebuild is reused unless Notion reports a newer edit, so searches don't
/// re-download every page. Pages with no text are indexed by title.
fn notion_text(
    state: &AppState,
    document: &DocumentRow,
    page_id: &str,
) -> Result<ExtractedDocument> {
    let cached: Option<String> = state.with_db(|conn| {
        Ok(conn
            .query_row(
                "SELECT content FROM document_embeddings
                 WHERE document_id = ?1 AND updated_at >= COALESCE(?2, 0)",
                rusqlite::params![document.id, document.modified_at],
                |row| row.get(0),
            )
            .optional()?)
    })?;

    let text = match cached {
        Some(text) => text,
        None => {
            let source = state.with_db(|conn| crate::db::get_source(conn, document.source_id))?;
            let workspace_id = source
                .config
                .get("workspace_id")
                .and_then(|v| v.as_str())
                .unwrap_or("default")
                .to_string();
            let connector = crate::connectors::NotionConnector::new(workspace_id);
            let page_id = page_id.to_string();
            block_on(async move { connector.download_text(&page_id).await })??
        }
    };

    Ok(ExtractedDocument {
        title: document.title.clone(),
        text: if text.trim().is_empty() {
            document.title.clone()
        } else {
            text
        },
    })
}

/// Runs a future to completion on a separate thread with its own runtime,
/// so it works whether or not the caller is already inside a Tokio runtime.
fn block_on<F>(future: F) -> Result<F::Output>
where
    F: std::future::Future + Send,
    F::Output: Send,
{
    std::thread::scope(|scope| {
        scope
            .spawn(|| {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()?;
                Ok(runtime.block_on(future))
            })
            .join()
            .map_err(|_| AppError::Internal("download thread panicked".into()))?
    })
}

// ---------- database bookkeeping ----------

/// Up to `limit` live documents that are new or changed since they were last
/// indexed.
fn load_pending(conn: &Connection, limit: usize) -> Result<Vec<DocumentRow>> {
    let mut stmt = conn.prepare(
        "SELECT id, source_id, external_id, title, modified_at, mime
         FROM documents
         WHERE deleted = 0 AND indexed_at IS NULL
         ORDER BY id
         LIMIT ?1",
    )?;
    let rows = stmt.query_map([limit as i64], document_row)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Deleted documents that are still in the search index.
fn load_removed(conn: &Connection) -> Result<Vec<i64>> {
    let mut stmt =
        conn.prepare("SELECT id FROM documents WHERE deleted = 1 AND indexed_at IS NOT NULL")?;
    let ids = stmt.query_map([], |row| row.get(0))?;
    Ok(ids.collect::<rusqlite::Result<_>>()?)
}

/// Number of documents waiting to be indexed, for the status display.
pub fn pending_count(conn: &Connection) -> Result<u32> {
    Ok(conn.query_row(
        "SELECT COUNT(*) FROM documents WHERE deleted = 0 AND indexed_at IS NULL",
        [],
        |row| row.get(0),
    )?)
}

/// IDs of a source's documents, so they can be dropped from the index before
/// the source and its rows are deleted.
pub fn source_document_ids(conn: &Connection, source_id: i64) -> Result<Vec<i64>> {
    let mut stmt = conn.prepare("SELECT id FROM documents WHERE source_id = ?1")?;
    let ids = stmt.query_map([source_id], |row| row.get(0))?;
    Ok(ids.collect::<rusqlite::Result<_>>()?)
}

/// Records that `document` is in the index. Does nothing if the document
/// changed while it was being indexed, so the newer version is picked up on
/// the next pass.
fn mark_indexed(conn: &Connection, document: &DocumentRow) -> Result<()> {
    conn.execute(
        "UPDATE documents SET indexed_at = unixepoch()
         WHERE id = ?1 AND indexed_at IS NULL AND modified_at IS ?2",
        rusqlite::params![document.id, document.modified_at],
    )?;
    Ok(())
}

/// Records that a deleted document is no longer in the index.
fn mark_removed(conn: &Connection, id: i64) -> Result<()> {
    conn.execute(
        "UPDATE documents SET indexed_at = NULL WHERE id = ?1 AND deleted = 1",
        [id],
    )?;
    Ok(())
}

// ---------- indexing ----------

/// Starts the background indexer thread.
pub fn spawn(app: tauri::AppHandle) -> std::io::Result<()> {
    std::thread::Builder::new()
        .name("indexer".into())
        .spawn(move || run(&app.state::<AppState>()))?;
    Ok(())
}

fn run(state: &AppState) {
    if let Err(e) = reconcile(state) {
        tracing::error!("could not check the search index: {e}");
    }

    // Documents whose text could not be downloaded, and when to try again.
    let mut retry_at: HashMap<i64, Instant> = HashMap::new();

    loop {
        match index_pass(state, &mut retry_at) {
            Ok(true) => continue, // a full batch: more may be waiting
            Ok(false) => {}
            Err(e) => tracing::error!("indexing pass failed: {e}"),
        }
        std::thread::sleep(POLL_INTERVAL);
    }
}

/// Re-indexes everything if the index and the database disagree: the index
/// is empty (first run, deleted, or recreated after a schema change) or no
/// document was ever marked indexed (versions before background indexing).
/// Cloud text is reused from the database, so nothing is downloaded again.
fn reconcile(state: &AppState) -> Result<()> {
    let marked: i64 = state.with_db(|conn| {
        Ok(conn.query_row(
            "SELECT COUNT(*) FROM documents WHERE indexed_at IS NOT NULL",
            [],
            |row| row.get(0),
        )?)
    })?;
    if marked > 0 && state.index.num_docs()? > 0 {
        return Ok(());
    }

    state.index.clear()?;
    state.with_db(|conn| {
        conn.execute("UPDATE documents SET indexed_at = NULL", [])?;
        Ok(())
    })?;
    tracing::info!("search index will be rebuilt in the background");
    Ok(())
}

/// Indexes up to `BATCH_SIZE` pending documents and drops deleted ones from
/// the index. Returns true if a full batch was processed.
fn index_pass(state: &AppState, retry_at: &mut HashMap<i64, Instant>) -> Result<bool> {
    let now = Instant::now();
    retry_at.retain(|_, at| *at > now);

    let removed = state.with_db(load_removed)?;
    let fetched = state.with_db(|conn| load_pending(conn, BATCH_SIZE + retry_at.len()))?;
    let pending: Vec<DocumentRow> = fetched
        .into_iter()
        .filter(|d| !retry_at.contains_key(&d.id))
        .take(BATCH_SIZE)
        .collect();

    if removed.is_empty() && pending.is_empty() {
        return Ok(false);
    }

    for &id in &removed {
        state.index.delete(id)?;
    }

    let batch_full = pending.len() == BATCH_SIZE;
    let mut done = Vec::with_capacity(pending.len());
    for document in pending {
        match index_document(state, &document) {
            Ok(()) => done.push(document),
            Err(error) => {
                tracing::warn!(
                    path = %document.path,
                    %error,
                    "could not index document; will retry in {} minutes",
                    RETRY_AFTER.as_secs() / 60
                );
                retry_at.insert(document.id, now + RETRY_AFTER);
            }
        }
    }

    // Commit before recording progress: a crash in between only means some
    // documents are indexed twice, never that one is marked but missing.
    state.index.commit()?;
    state.with_db(|conn| {
        for &id in &removed {
            mark_removed(conn, id)?;
        }
        for document in &done {
            mark_indexed(conn, document)?;
        }
        Ok(())
    })?;

    tracing::info!(
        indexed = done.len(),
        removed = removed.len(),
        "search index updated"
    );
    Ok(batch_full)
}

/// Reads a document's text. `Ok(None)` means there is nothing to index (a
/// missing, unsupported or unreadable local file; it is tried again when it
/// changes). An error means a download failed and should be retried.
fn read_text(state: &AppState, document: &DocumentRow) -> Result<Option<ExtractedDocument>> {
    if let Some(file_id) = document.path.strip_prefix("gdrive:") {
        return google_drive_text(state, document, file_id).map(Some);
    }
    if let Some(page_id) = document.path.strip_prefix("notion:") {
        return notion_text(state, document, page_id).map(Some);
    }

    let path = Path::new(&document.path);
    if !path.is_file() || !crate::extract::TextExtractor.supports(path) {
        tracing::debug!(path = %path.display(), "not indexed: missing or unsupported file");
        return Ok(None);
    }
    match crate::extract::extract(path) {
        Ok(extracted) => Ok(Some(extracted)),
        Err(error) => {
            tracing::warn!(path = %path.display(), %error, "could not extract text");
            Ok(None)
        }
    }
}

fn index_document(state: &AppState, document: &DocumentRow) -> Result<()> {
    let Some(extracted) = read_text(state, document)? else {
        return Ok(());
    };

    // Files without text (e.g. scanned PDFs) are still indexed by name.
    let indexed = IndexedDocument {
        document_id: document.id,
        source_id: document.source_id,
        title: if extracted.title.is_empty() {
            document.title.clone()
        } else {
            extracted.title
        },
        content: extracted.text,
        location: document.path.clone(),
        modified_at: document.modified_at,
    };

    state.index.upsert(&indexed)?;

    // Keyword search works without an embedding, so a failure here is only logged.
    if let Err(error) = save_embedding(state, &indexed) {
        tracing::warn!(document_id = indexed.document_id, %error, "could not save embedding");
    }

    tracing::debug!(
        document_id = indexed.document_id,
        path = %indexed.location,
        chars = indexed.content.len(),
        "document indexed"
    );
    Ok(())
}

/// Saves the text and its embedding for semantic search. The embedding is
/// only recomputed when the text changed.
fn save_embedding(state: &AppState, indexed: &IndexedDocument) -> Result<()> {
    let document_id = indexed.document_id;

    // Same text as last time: keep the vector, refresh the timestamp so the
    // cached cloud text counts as current.
    let unchanged = state.with_db(|conn| {
        Ok(conn.execute(
            "UPDATE document_embeddings SET updated_at = unixepoch()
             WHERE document_id = ?1 AND content = ?2",
            rusqlite::params![document_id, indexed.content],
        )? > 0)
    })?;
    if unchanged {
        return Ok(());
    }

    // Keep the text bounded for the embedding model.
    let input = format!(
        "{}\n\n{}",
        indexed.title,
        indexed.content.chars().take(6000).collect::<String>()
    );
    let vector = crate::embed::embed_text(&input)?;
    let blob = crate::embed::vector_to_blob(&vector);
    let dimensions = vector.len() as i64;

    state.with_db(|conn| {
        conn.execute(
            "INSERT INTO document_embeddings
                (document_id, content, embedding, dimensions, updated_at)
             VALUES (?1, ?2, ?3, ?4, unixepoch())
             ON CONFLICT(document_id) DO UPDATE SET
                content = excluded.content,
                embedding = excluded.embedding,
                dimensions = excluded.dimensions,
                updated_at = unixepoch()",
            rusqlite::params![document_id, indexed.content, blob, dimensions],
        )?;
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{self, DocumentUpsert};

    fn conn_with_source() -> Connection {
        let conn = db::open_in_memory().unwrap();
        conn.execute("INSERT INTO sources (id, kind) VALUES (1, 'local')", [])
            .unwrap();
        conn
    }

    fn upsert(conn: &Connection, path: &str, modified_at: i64) {
        let doc = DocumentUpsert {
            external_id: path,
            title: path,
            mime: None,
            modified_at: Some(modified_at),
        };
        db::upsert_document(conn, 1, &doc, 100).unwrap();
    }

    fn pending_paths(conn: &Connection) -> Vec<String> {
        load_pending(conn, 100)
            .unwrap()
            .into_iter()
            .map(|d| d.path)
            .collect()
    }

    #[test]
    fn indexed_documents_are_no_longer_pending() {
        let conn = conn_with_source();
        upsert(&conn, "/a.txt", 10);
        upsert(&conn, "/b.txt", 10);
        assert_eq!(pending_count(&conn).unwrap(), 2);

        let a = load_pending(&conn, 1).unwrap().remove(0);
        mark_indexed(&conn, &a).unwrap();

        assert_eq!(pending_paths(&conn), ["/b.txt"]);
        assert_eq!(pending_count(&conn).unwrap(), 1);
    }

    #[test]
    fn unchanged_documents_are_not_indexed_again() {
        let conn = conn_with_source();
        upsert(&conn, "/a.txt", 10);
        let a = load_pending(&conn, 1).unwrap().remove(0);
        mark_indexed(&conn, &a).unwrap();

        upsert(&conn, "/a.txt", 10); // seen again by a crawl, not modified
        assert!(pending_paths(&conn).is_empty());

        upsert(&conn, "/a.txt", 20); // modified
        assert_eq!(pending_paths(&conn), ["/a.txt"]);
    }

    #[test]
    fn a_document_changed_while_indexing_stays_pending() {
        let conn = conn_with_source();
        upsert(&conn, "/a.txt", 10);
        let old = load_pending(&conn, 1).unwrap().remove(0);

        upsert(&conn, "/a.txt", 20); // edited before the indexer finished
        mark_indexed(&conn, &old).unwrap();

        assert_eq!(pending_paths(&conn), ["/a.txt"]);
    }

    #[test]
    fn deleted_documents_are_removed_once() {
        let conn = conn_with_source();
        upsert(&conn, "/a.txt", 10);
        let a = load_pending(&conn, 1).unwrap().remove(0);
        mark_indexed(&conn, &a).unwrap();

        db::mark_deleted_under(&conn, 1, "/a.txt", '/').unwrap();
        assert_eq!(load_removed(&conn).unwrap(), [a.id]);
        assert_eq!(pending_count(&conn).unwrap(), 0);

        mark_removed(&conn, a.id).unwrap();
        assert!(load_removed(&conn).unwrap().is_empty());
    }

    #[test]
    fn source_document_ids_lists_only_that_source() {
        let conn = conn_with_source();
        conn.execute("INSERT INTO sources (id, kind) VALUES (2, 'local')", [])
            .unwrap();
        upsert(&conn, "/a.txt", 10);
        db::upsert_document(
            &conn,
            2,
            &DocumentUpsert {
                external_id: "/b.txt",
                title: "b",
                mime: None,
                modified_at: None,
            },
            100,
        )
        .unwrap();

        assert_eq!(source_document_ids(&conn, 1).unwrap().len(), 1);
        assert_eq!(source_document_ids(&conn, 2).unwrap().len(), 1);
    }
}
