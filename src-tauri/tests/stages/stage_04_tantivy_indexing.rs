use crate::support;
use support::*;

#[test]
fn indexes_document_and_replaces_existing_document_id() {
    let tmp = temp();
    let idx = index(tmp.path().join("index"));
    let path = tmp.path().join("notes.txt");
    idx.upsert(&indexed(1, 7, "First title", "alpha content", &path))
        .unwrap();
    idx.commit().unwrap();
    idx.upsert(&indexed(1, 7, "Updated title", "bravo content", &path))
        .unwrap();
    idx.commit().unwrap();
    let (hits, total, _) = idx.search("bravo", 10).unwrap();
    assert_eq!(total, 1);
    assert_eq!(hits[0].title, "Updated title");
    assert_eq!(idx.search("alpha", 10).unwrap().1, 0);
}
