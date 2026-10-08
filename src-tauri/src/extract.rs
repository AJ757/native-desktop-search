//! Text extraction from files.
//!
//! Defines the `Extractor` trait: which file types an extractor handles and
//! how it turns bytes into text, a title and metadata. One implementation
//! per format (plain text, Markdown, PDF, Office files, HTML).

pub trait Extractor: Send + Sync {
    // which file types it handles, bytes -> text + title + metadata
}