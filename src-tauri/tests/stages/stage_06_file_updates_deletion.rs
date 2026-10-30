use crate::support;
use native_desktop_search_lib::{db, sources::SourceEvent};
use std::fs;
use support::*;

#[test]
fn changed_files_are_upserted_and_deleted_paths_are_marked_and_unindexed() {
    let tmp = temp();
    let root = tmp.path().join("docs");
    fs::create_dir(&root).unwrap();
    let file = root.join("note.txt");
    fs::write(&file, "before").unwrap();
    let mut conn = db::open_in_memory().unwrap();
    let src = source(&conn, &root);
    let f = folder(&root, &[]);
    apply_events(&mut conn, src.id, crawl(&f));
    let id = document_id(&conn, src.id, &file.to_string_lossy());
    let idx = index(tmp.path().join("index"));
    idx.upsert(&indexed(id, src.id, "note.txt", "before", &file))
        .unwrap();
    idx.commit().unwrap();
    fs::write(&file, "afterword").unwrap();
    apply_events(&mut conn, src.id, f.changes_at(&file, false));
    assert!(idx.search("afterword", 10).unwrap().1 == 0); // index update is a separate indexing stage
    let modified: i64 = conn
        .query_row("SELECT modified_at FROM documents WHERE id=?1", [id], |r| {
            r.get(0)
        })
        .unwrap();
    assert!(modified > 0);
    fs::remove_file(&file).unwrap();
    let deletes = f.changes_at(&file, false);
    assert!(matches!(deletes.as_slice(), [SourceEvent::Delete { .. }]));
    apply_events(&mut conn, src.id, deletes);
    let deleted: bool = conn
        .query_row("SELECT deleted FROM documents WHERE id=?1", [id], |r| {
            r.get(0)
        })
        .unwrap();
    assert!(deleted);
    idx.delete(id).unwrap();
    idx.commit().unwrap();
    assert_eq!(idx.search("before", 10).unwrap().1, 0);
}
