use crate::support;
use native_desktop_search_lib::extract::{Extractor, TextExtractor};
use std::fs;
use support::*;

#[test]
fn empty_text_files_extract_as_empty_text() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("empty.txt");
    fs::write(&path, []).unwrap();
    let document = TextExtractor.extract(&path).unwrap();
    assert_eq!(document.title, "empty.txt");
    assert!(document.text.is_empty());

    let index = index(tmp.path().join("index"));
    index
        .upsert(&indexed(1, 1, &document.title, &document.text, &path))
        .unwrap();
    index.commit().unwrap();
    assert_eq!(index.search("empty", 10).unwrap().1, 1);
}
