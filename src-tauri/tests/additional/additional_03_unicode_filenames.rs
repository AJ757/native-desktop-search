use crate::support;
use native_desktop_search_lib::sources::SourceEvent;
use std::fs;
use support::*;

#[test]
fn crawler_and_extractor_preserve_unicode_filenames() {
    let tmp = temp();
    let path = tmp.path().join("café_日本語.txt");
    fs::write(&path, "unicode content").unwrap();
    let events = crawl(&folder(tmp.path(), &[]));
    assert!(
        matches!(&events[..], [SourceEvent::Upsert { title, .. }] if title == "café_日本語.txt")
    );
    let extracted = native_desktop_search_lib::extract::extract(&path).unwrap();
    assert_eq!(extracted.title, "café_日本語.txt");
}
