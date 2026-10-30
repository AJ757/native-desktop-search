use crate::support;
use native_desktop_search_lib::{
    db,
    sources::SourceEvent,
    sync::{self, SyncMsg},
};
use std::{
    fs,
    time::{SystemTime, UNIX_EPOCH},
};
use support::*;

#[test]
fn upserts_changes_deletes_and_sweeps_files_missing_from_full_crawl() {
    let tmp = temp();
    let root = tmp.path().join("docs");
    fs::create_dir(&root).unwrap();
    let a = root.join("a.txt");
    let b = root.join("b.txt");
    fs::write(&a, "a").unwrap();
    fs::write(&b, "b").unwrap();
    let mut conn = db::open_in_memory().unwrap();
    let src = source(&conn, &root);
    apply_events(&mut conn, src.id, crawl(&folder(&root, &[])));
    assert_eq!(db::count_documents(&conn).unwrap(), 2);
    let start = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    fs::remove_file(&b).unwrap();
    fs::write(&a, "changed").unwrap();
    let events = folder(&root, &[]).changes_at(&a, false);
    assert!(matches!(events.as_slice(), [SourceEvent::Upsert { .. }]));
    apply_events(&mut conn, src.id, events);
    conn.execute(
        "UPDATE documents SET seen_at=?1 WHERE external_id=?2",
        rusqlite::params![start + 1, a.to_string_lossy().as_ref()],
    )
    .unwrap();
    conn.execute(
        "UPDATE documents SET seen_at=?1 WHERE external_id=?2",
        rusqlite::params![start - 1, b.to_string_lossy().as_ref()],
    )
    .unwrap();
    let a_seen: Option<i64> = conn
        .query_row(
            "SELECT seen_at FROM documents WHERE external_id=?1",
            [&a.to_string_lossy()],
            |r| r.get(0),
        )
        .unwrap();
    assert!(a_seen.unwrap() > 0);
    sync::apply(
        &mut conn,
        SyncMsg::CrawlDone {
            source_id: src.id,
            started_at: start,
        },
    )
    .unwrap();
    assert_eq!(db::count_documents(&conn).unwrap(), 1);
}
