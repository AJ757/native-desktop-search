use std::path::Path;

use rusqlite::Connection;

use crate::app::AppState;
use crate::error::Result;
use crate::extract::Extractor;
use crate::index::IndexedDocument;

struct DocumentRow {
    id: i64,
    source_id: i64,
    path: String,
    title: String,
    modified_at: Option<i64>,
}

fn load_documents(conn: &Connection) -> Result<Vec<DocumentRow>> {
    let mut stmt = conn.prepare(
        "SELECT id, source_id, external_id, title, modified_at
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
        })
    })?;

    let mut documents = Vec::new();

    for row in rows {
        documents.push(row?);
    }

    Ok(documents)
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

        let extracted = match crate::extract::extract(path) {
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
        };

        extraction_count += 1;

        tracing::info!(
            path = %path.display(),
            extracted_chars = extracted.text.chars().count(),
            extracted_title = %extracted.title,
            "CHECK 7: TEXT EXTRACTION COMPLETED"
        );

        // 7. Check extracted text.
        if extracted.text.trim().is_empty() {
            tracing::warn!(
                path = %path.display(),
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
