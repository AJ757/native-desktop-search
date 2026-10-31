use crate::support;
use std::sync::Arc;
use support::*;

#[test]
fn concurrent_indexing_and_search_complete_without_losing_documents() {
    let tmp = temp();
    let idx = Arc::new(index(tmp.path().join("index")));
    let mut workers = Vec::new();
    for worker in 0..4 {
        let idx = Arc::clone(&idx);
        workers.push(std::thread::spawn(move || {
            for offset in 0..5 {
                let id = worker * 5 + offset + 1;
                let path = std::path::PathBuf::from(format!("/parallel/{id}.txt"));
                idx.upsert(&indexed(
                    id,
                    1,
                    &format!("parallel {id}"),
                    "concurrency marker",
                    &path,
                ))
                .unwrap();
                let _ = idx.search("concurr", 20).unwrap();
            }
        }));
    }
    for worker in workers {
        worker.join().unwrap();
    }
    idx.commit().unwrap();
    let (_, total, _) = idx.search("marker", 20).unwrap();
    assert_eq!(total, 20);
}
