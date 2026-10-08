//! SQLite access: connection setup, migrations and queries.
//!
//! SQLite is the source of truth. The Tantivy index and the vector store
//! must always be rebuildable from what is stored here.