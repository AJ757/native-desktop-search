use crate::support;
use native_desktop_search_lib::db;
use std::fs;
use support::*;

#[test]
fn same_filename_in_different_directories_gets_distinct_rows() {
    let tmp = temp();
    let root = tmp.path().join("docs");
    fs::create_dir_all(root.join("one")).unwrap();
    fs::create_dir_all(root.join("two")).unwrap();
    fs::write(root.join("one/same.txt"), "first").unwrap();
    fs::write(root.join("two/same.txt"), "second").unwrap();
    let mut conn = db::open_in_memory().unwrap();
    let src = source(&conn, &root);
    apply_events(&mut conn, src.id, crawl(&folder(&root, &[])));
    assert_eq!(db::count_documents(&conn).unwrap(), 2);
    let paths: i64 = conn
        .query_row(
            "SELECT COUNT(DISTINCT external_id) FROM documents WHERE title='same.txt'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(paths, 2);
}
