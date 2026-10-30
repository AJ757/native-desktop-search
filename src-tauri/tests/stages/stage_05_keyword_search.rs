use crate::support;
use support::*;

#[test]
fn keyword_search_matches_words_and_prefixes_and_honors_limit() {
    let tmp = temp();
    let idx = index(tmp.path().join("index"));
    for (id, title, body) in [
        (1, "Lecture one", "rust systems"),
        (2, "Other", "lecture notes"),
    ] {
        idx.upsert(&indexed(
            id,
            1,
            title,
            body,
            &tmp.path().join(format!("{id}.txt")),
        ))
        .unwrap();
    }
    idx.commit().unwrap();
    let (hits, total, _) = idx.search("lec", 1).unwrap();
    assert_eq!(total, 2);
    assert_eq!(hits.len(), 1);
    assert_eq!(idx.search("lec rust", 10).unwrap().1, 1);
    assert_eq!(idx.search("absent", 10).unwrap().1, 0);
}
