use crate::support;
use native_desktop_search_lib::{
    db,
    extract::{Extractor, TextExtractor},
    index::IndexedDocument,
    sources::SourceEvent,
};
use std::fs;
use support::*;

#[test]
fn crawls_persists_extracts_indexes_and_finds_a_file() {
    let tmp = temp();
    let root = tmp.path().join("docs");
    fs::create_dir(&root).unwrap();
    let path = root.join("project-plan.txt");
    fs::write(&path, "quartz migration checklist").unwrap();
    let mut conn = db::open_in_memory().unwrap();
    let src = source(&conn, &root);
    let events = crawl(&folder(&root, &[]));
    assert_eq!(events.len(), 1);
    apply_events(&mut conn, src.id, events);
    let id = document_id(&conn, src.id, &path.to_string_lossy());
    let extracted = TextExtractor.extract(&path).unwrap();
    let idx = index(tmp.path().join("index"));
    idx.upsert(&IndexedDocument {
        document_id: id,
        source_id: src.id,
        title: extracted.title,
        content: extracted.text,
        location: path.to_string_lossy().into_owned(),
        modified_at: Some(1),
    })
    .unwrap();
    idx.commit().unwrap();
    let (hits, total, _) = idx.search("quartz", 10).unwrap();
    assert_eq!(total, 1);
    assert_eq!(hits[0].location, path.to_string_lossy());
    assert!(matches!(
        crawl(&folder(&root, &[])).as_slice(),
        [SourceEvent::Upsert { .. }]
    ));
}
