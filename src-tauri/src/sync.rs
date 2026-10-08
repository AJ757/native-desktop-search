//! Synchronisation and ingestion.
//!
//! Runs the ingest queue and the per-source sync tasks (initial crawl,
//! file watching, scheduled cloud polling) and keeps cursors in `sync_state`.