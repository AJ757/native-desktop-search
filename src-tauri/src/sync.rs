//! Synchronisation and ingestion.
//!
//! Runs the ingest queue and the per-source sync tasks (initial crawl,
//! file watching, scheduled cloud polling) and keeps cursors in `sync_state`.

use crate::types::IndexStatus;

pub fn index_status() -> IndexStatus {
    IndexStatus { total_documents: 0, queued: 0, is_indexing: false }
}