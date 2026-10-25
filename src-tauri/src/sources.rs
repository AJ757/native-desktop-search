//! Data sources: local folders and cloud connectors.
//!
//! Defines the `Source` trait. Every source reports changes since a cursor
//! (upsert or delete events), fetches item content and provides an open
//! target (file path or URL), so the ingest pipeline treats all of them
//! the same way.

use async_trait::async_trait;

#[async_trait]
pub trait Source: Send + Sync {
    // list changes since a cursor, fetch content, open target, kind...
}
