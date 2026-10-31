use crate::support;
use support::*;

#[test]
fn empty_and_punctuation_queries_return_safely() {
    let tmp = temp();
    let idx = index(tmp.path().join("index"));
    idx.upsert(&indexed(
        1,
        1,
        "punctuation document",
        "ordinary body",
        tmp.path(),
    ))
    .unwrap();
    idx.commit().unwrap();
    let (hits, total, _) = idx.search("   ", 10).unwrap();
    assert!(hits.is_empty());
    assert_eq!(total, 0);
    for query in ["!!!", "()[]{}", "\\", "\"quoted\""] {
        let _ = idx.search(query, 10).unwrap();
    }
}
