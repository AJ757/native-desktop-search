use std::path::Path;

use rusqlite::{Connection, OptionalExtension};

use crate::app::AppState;
use crate::error::{AppError, Result};
use crate::extract::{ExtractedDocument, Extractor};
use crate::index::IndexedDocument;

struct DocumentRow {
    id: i64,
    source_id: i64,
    path: String,
    title: String,
    modified_at: Option<i64>,
    mime: Option<String>,
}

fn load_documents(conn: &Connection) -> Result<Vec<DocumentRow>> {
    let mut stmt = conn.prepare(
        "SELECT id, source_id, external_id, title, modified_at, mime
         FROM documents
         WHERE deleted = 0
         ORDER BY id",
    )?;

    let rows = stmt.query_map([], |row| {
        Ok(DocumentRow {
            id: row.get(0)?,
            source_id: row.get(1)?,
            path: row.get(2)?,
            title: row.get(3)?,
            modified_at: row.get(4)?,
            mime: row.get(5)?,
        })
    })?;

    let mut documents = Vec::new();

    for row in rows {
        documents.push(row?);
    }

    Ok(documents)
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

pub fn rebuild(state: &AppState) -> Result<()> {
    tracing::info!("========== INDEX REBUILD START ==========");

    // 1. Load documents from SQLite.
    let documents = state.with_db(load_documents)?;

    tracing::info!(
        total_documents = documents.len(),
        "CHECK 1: DOCUMENTS LOADED FROM SQLITE"
    );

    // 2. Clear the previous Tantivy index.
    state.index.clear()?;

    tracing::info!("CHECK 2: OLD INDEX CLEARED");

    let mut indexed_count = 0usize;
    let mut skipped_count = 0usize;
    let mut extraction_count = 0usize;

    // 3. Process every document.
    for document in documents {
        tracing::info!(
            document_id = document.id,
            source_id = document.source_id,
            path = %document.path,
            title = %document.title,
            "CHECK 3: PROCESSING DOCUMENT"
        );

        // Google Drive documents are downloaded instead of read from disk.
        let extracted = if let Some(file_id) = document.path.strip_prefix("gdrive:") {
            match google_drive_text(state, &document, file_id) {
                Ok(value) => value,
                Err(error) => {
                    tracing::error!(
                        path = %document.path,
                        error = %error,
                        "CHECK FAILED: GOOGLE DRIVE DOWNLOAD ERROR"
                    );

                    skipped_count += 1;
                    continue;
                }
            }
        }
        
        else if let Some(page_id) = document.path.strip_prefix("notion:") {
            match notion_text(state, &document, page_id) {
                Ok(value) => value,
                Err(error) => {
                    tracing::error!(
                        path = %document.path,
                        error = %error,
                        "CHECK FAILED: NOTION DOWNLOAD ERROR"
                    );

                    skipped_count += 1;
                    continue;
                }
            }
        }
        
        else {
            let path = Path::new(&document.path);

            // 4. Check whether the file exists.
            if !path.is_file() {
                tracing::warn!(
                    path = %path.display(),
                    "CHECK FAILED: FILE DOES NOT EXIST"
                );

                skipped_count += 1;
                continue;
            }

            tracing::info!(
                path = %path.display(),
                "CHECK 4: FILE EXISTS"
            );

            // 5. Check whether the extractor supports the extension.
            if !crate::extract::TextExtractor.supports(path) {
                tracing::warn!(
                    path = %path.display(),
                    extension = ?path.extension(),
                    "CHECK FAILED: UNSUPPORTED FILE EXTENSION"
                );

                skipped_count += 1;
                continue;
            }

            tracing::info!(
                path = %path.display(),
                "CHECK 5: FILE EXTENSION SUPPORTED"
            );

            // 6. Extract text.
            tracing::info!(
                path = %path.display(),
                "CHECK 6: STARTING TEXT EXTRACTION"
            );

            match crate::extract::extract(path) {
                Ok(value) => value,
                Err(error) => {
                    tracing::error!(
                        path = %path.display(),
                        error = %error,
                        "CHECK FAILED: TEXT EXTRACTION ERROR"
                    );

                    skipped_count += 1;
                    continue;
                }
            }
        };

        extraction_count += 1;

        tracing::info!(
            path = %document.path,
            extracted_chars = extracted.text.chars().count(),
            extracted_title = %extracted.title,
            "CHECK 7: TEXT EXTRACTION COMPLETED"
        );

        // 7. Check extracted text.
        if extracted.text.trim().is_empty() {
            tracing::warn!(
                path = %document.path,
                "CHECK FAILED: EXTRACTED TEXT IS EMPTY"
            );

            skipped_count += 1;
            continue;
        }

        // 8. Construct the Tantivy document.
        let indexed = IndexedDocument {
            document_id: document.id,
            source_id: document.source_id,
            title: if extracted.title.is_empty() {
                document.title
            } else {
                extracted.title
            },
            content: extracted.text,
            location: document.path.clone(),
            modified_at: document.modified_at,
        };

        tracing::info!(
            document_id = indexed.document_id,
            path = %indexed.location,
            "CHECK 8: SENDING DOCUMENT TO TANTIVY"
        );

        // 9. Add the document to Tantivy.
        // 9. Generate and save a local embedding.
        // Keep the text bounded for the embedding model.
        let embedding_input = format!(
            "{}\n\n{}",
            indexed.title,
            indexed.content.chars().take(6000).collect::<String>()
        );

        let embedding = match crate::embed::embed_text(&embedding_input) {
            Ok(vector) => Some(vector),
            Err(error) => {
                tracing::error!(
                    document_id = indexed.document_id,
                    error = %error,
                    "Could not generate embedding; keeping keyword indexing available"
                );
                None
            }
        };

        // Add the document to Tantivy regardless of embedding success.
        if let Err(error) = state.index.upsert(&indexed) {
            tracing::error!(
                document_id = indexed.document_id,
                path = %indexed.location,
                error = %error,
                "CHECK FAILED: TANTIVY UPSERT ERROR"
            );

            return Err(error);
        }

        // Save the extracted text and vector for semantic search.
        if let Some(vector) = embedding {
            let content = indexed.content.clone();
            let vector_blob = crate::embed::vector_to_blob(&vector);
            let dimensions = vector.len() as i64;
            let document_id = indexed.document_id;

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
                    rusqlite::params![document_id, content, vector_blob, dimensions],
                )?;

                Ok(())
            })?;
        }

        indexed_count += 1;

        tracing::info!(
            document_id = indexed.document_id,
            "CHECK 9: DOCUMENT ADDED TO TANTIVY"
        );
    }

    // 10. Commit the index.
    tracing::info!(
        indexed = indexed_count,
        skipped = skipped_count,
        extracted = extraction_count,
        "CHECK 10: COMMITTING TANTIVY INDEX"
    );

    state.index.commit()?;

    tracing::info!("CHECK 11: TANTIVY COMMIT COMPLETED");

    // 11. Final summary.
    tracing::info!(
        total_documents = indexed_count + skipped_count,
        extracted = extraction_count,
        indexed = indexed_count,
        skipped = skipped_count,
        "========== INDEX REBUILD FINISHED =========="
    );

    Ok(())
}
