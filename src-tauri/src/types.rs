//! Shared domain types: `Document`, `Chunk`, `SearchHit`, `SourceKind`
//! and the command request/response structs.
//!
//! TypeScript definitions are generated from these (via ts-rs) so the Rust
//! and React sides never drift apart.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

// ---------- enums ----------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum SourceKind {
    Local,
    Obsidian,
    GoogleDrive,
    OneDrive,
    Notion,
}

impl SourceKind {
    /// Value stored in `sources.kind`. Must equal the serde form.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Obsidian => "obsidian",
            Self::GoogleDrive => "google_drive",
            Self::OneDrive => "one_drive",
            Self::Notion => "notion",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum SourceStatus {
    Idle,
    Syncing,
    Error,
}

impl SourceStatus {
    /// Value stored in `sources.status`. Must equal the serde form.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Syncing => "syncing",
            Self::Error => "error",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum SearchMode {
    Keyword,
    Semantic,
    Hybrid,
}

// ---------- stored entities ----------
// Timestamps are Unix seconds (matches SQLite's unixepoch()).

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Source {
    #[ts(type = "number")]
    pub id: i64,
    pub kind: SourceKind,
    #[ts(type = "Record<string, unknown>")]
    pub config: serde_json::Value,
    pub status: SourceStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Document {
    #[ts(type = "number")]
    pub id: i64,
    #[ts(type = "number")]
    pub source_id: i64,
    pub external_id: String,
    pub title: String,
    pub mime: Option<String>,
    #[ts(type = "number | null")]
    pub modified_at: Option<i64>,
    pub content_hash: Option<String>,
    #[ts(type = "number | null")]
    pub indexed_at: Option<i64>,
    pub deleted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Chunk {
    #[ts(type = "number")]
    pub id: i64,
    #[ts(type = "number")]
    pub document_id: i64,
    pub ordinal: u32,
    pub start_offset: u32,
    pub end_offset: u32,
}

// ---------- search contract ----------

#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SearchFilters {
    #[serde(default)]
    #[ts(type = "number[]")]
    pub source_ids: Vec<i64>,
    #[serde(default)]
    pub file_types: Vec<String>,
    #[ts(type = "number | null")]
    pub modified_after: Option<i64>,
    #[ts(type = "number | null")]
    pub modified_before: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SearchRequest {
    pub query: String,
    pub filters: SearchFilters,
    pub limit: u32,
    pub mode: SearchMode,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SearchHit {
    #[ts(type = "number")]
    pub document_id: i64,
    pub title: String,
    pub snippet: String,
    #[ts(type = "number")]
    pub source_id: i64,
    pub source_kind: SourceKind,
    /// File path or URL used to open the result.
    pub location: String,
    #[ts(type = "number | null")]
    pub modified_at: Option<i64>,
    pub score: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SearchResponse {
    pub hits: Vec<SearchHit>,
    pub total: u32,
    pub took_ms: u32,
}

// ---------- other command payloads ----------

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AddSourceRequest {
    pub kind: SourceKind,
    #[ts(type = "Record<string, unknown>")]
    pub config: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct IndexStatus {
    pub total_documents: u32,
    pub queued: u32,
    pub is_indexing: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_kind_as_str_matches_serde() {
        for kind in [
            SourceKind::Local,
            SourceKind::Obsidian,
            SourceKind::GoogleDrive,
            SourceKind::OneDrive,
            SourceKind::Notion,
        ] {
            let json = serde_json::to_string(&kind).unwrap();
            assert_eq!(json, format!("\"{}\"", kind.as_str()));
        }
    }

    #[test]
    fn source_status_as_str_matches_serde() {
        for status in [
            SourceStatus::Idle,
            SourceStatus::Syncing,
            SourceStatus::Error,
        ] {
            let json = serde_json::to_string(&status).unwrap();
            assert_eq!(json, format!("\"{}\"", status.as_str()));
        }
    }

    #[test]
    fn search_hit_uses_camel_case_on_the_wire() {
        let hit = SearchHit {
            document_id: 1,
            title: "t".into(),
            snippet: "s".into(),
            source_id: 2,
            source_kind: SourceKind::Local,
            location: "C:/a.txt".into(),
            modified_at: None,
            score: 0.5,
        };
        let v = serde_json::to_value(&hit).unwrap();
        assert!(v.get("documentId").is_some());
        assert!(v.get("document_id").is_none());
        assert!(v["modifiedAt"].is_null());
        assert_eq!(v["sourceKind"], "local");
    }
}
