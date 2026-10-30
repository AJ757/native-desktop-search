use crate::support;
use native_desktop_search_lib::db;
use support::*;

#[test]
fn source_and_document_rows_round_trip_through_sqlite() {
    let tmp = temp();
    let conn = db::open_in_memory().unwrap();
    let src = source(&conn, tmp.path());
    upsert(&conn, src.id, "note.txt", "note.txt", 100, 200);
    assert_eq!(db::count_documents(&conn).unwrap(), 1);
    let loaded = db::get_source(&conn, src.id).unwrap();
    assert_eq!(loaded.config["path"], tmp.path().to_string_lossy().as_ref());
    let title: String = conn
        .query_row(
            "SELECT title FROM documents WHERE external_id='note.txt'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(title, "note.txt");
}
