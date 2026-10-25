//! Keyword search index (Tantivy).
//!
//! Owns the index schema, the single long-lived writer and the reader.
//! Documents are committed in batches, not one at a time.
