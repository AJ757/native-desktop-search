#![allow(dead_code)]

use native_desktop_search_lib::{
    db::{self, DocumentUpsert},
    index::{IndexedDocument, SearchIndex},
    sources::{ExclusionFilter, LocalFolder, Source as _, SourceEvent},
    sync::{self, SyncMsg},
    types::{AddSourceRequest, Source, SourceKind},
};
use rusqlite::{params, Connection};
use std::{
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
};

pub fn temp() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}
pub fn source(conn: &Connection, root: &Path) -> Source {
    db::insert_source(
        conn,
        &AddSourceRequest {
            kind: SourceKind::Local,
            config: serde_json::json!({"path": root}),
        },
    )
    .unwrap()
}
pub fn folder(root: &Path, patterns: &[&str]) -> LocalFolder {
    LocalFolder::new(
        SourceKind::Local,
        root.to_path_buf(),
        ExclusionFilter::new(
            &patterns
                .iter()
                .map(|p| (*p).to_string())
                .collect::<Vec<_>>(),
            vec![],
        ),
    )
}
pub fn crawl(folder: &LocalFolder) -> Vec<SourceEvent> {
    let mut events = Vec::new();
    assert!(folder
        .crawl(&AtomicBool::new(false), &mut |batch| events.extend(batch))
        .unwrap());
    events
}
pub fn apply_events(conn: &mut Connection, source_id: i64, events: Vec<SourceEvent>) {
    sync::apply(conn, SyncMsg::Events { source_id, events }).unwrap();
}
pub fn indexed(
    id: i64,
    source_id: i64,
    title: &str,
    content: &str,
    path: &Path,
) -> IndexedDocument {
    IndexedDocument {
        document_id: id,
        source_id,
        title: title.into(),
        content: content.into(),
        location: path.to_string_lossy().into_owned(),
        modified_at: Some(10),
    }
}
pub fn document_id(conn: &Connection, source_id: i64, external_id: &str) -> i64 {
    conn.query_row(
        "SELECT id FROM documents WHERE source_id=?1 AND external_id=?2",
        params![source_id, external_id],
        |r| r.get(0),
    )
    .unwrap()
}
pub fn upsert(
    conn: &Connection,
    source_id: i64,
    external_id: &str,
    title: &str,
    modified_at: i64,
    seen_at: i64,
) {
    db::upsert_document(
        conn,
        source_id,
        &DocumentUpsert {
            external_id,
            title,
            mime: Some("text/plain"),
            modified_at: Some(modified_at),
        },
        seen_at,
    )
    .unwrap();
}
pub fn index(path: PathBuf) -> SearchIndex {
    SearchIndex::open(path).unwrap()
}
