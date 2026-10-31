use crate::support;
use native_desktop_search_lib::db;
use std::fs;
use support::*;

#[test]
fn sqlite_and_tantivy_results_survive_close_and_reopen() {
    let tmp = temp();
    let docs = tmp.path().join("docs");
    fs::create_dir(&docs).unwrap();
    let path = docs.join("durable.txt");
    fs::write(&path, "persistent walnut").unwrap();
    let db_path = tmp.path().join("app.sqlite");
    let index_path = tmp.path().join("index");
    let source_id;
    {
        let conn = db::open(&db_path).unwrap();
        let src = source(&conn, &docs);
        source_id = src.id;
        upsert(
            &conn,
            src.id,
            &path.to_string_lossy(),
            "durable.txt",
            10,
            20,
        );
        let id = document_id(&conn, src.id, &path.to_string_lossy());
        let idx = index(index_path.clone());
        idx.upsert(&indexed(
            id,
            src.id,
            "durable.txt",
            "persistent walnut",
            &path,
        ))
        .unwrap();
        idx.commit().unwrap();
    }
    let conn = db::open(&db_path).unwrap();
    assert_eq!(db::count_documents(&conn).unwrap(), 1);
    assert_eq!(db::get_source(&conn, source_id).unwrap().id, source_id);
    let idx = index(index_path);
    let (hits, total, _) = idx.search("walnut", 10).unwrap();
    assert_eq!(total, 1);
    assert_eq!(hits[0].title, "durable.txt");
}
