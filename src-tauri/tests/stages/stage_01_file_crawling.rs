use crate::support;
use native_desktop_search_lib::sources::SourceEvent;
use std::fs;
use support::*;

#[test]
fn crawls_nested_files_and_applies_hidden_default_and_custom_exclusions() {
    let tmp = temp();
    let root = tmp.path();
    fs::create_dir_all(root.join("nested")).unwrap();
    fs::write(root.join("keep.txt"), "ok").unwrap();
    fs::write(root.join("nested/also.md"), "ok").unwrap();
    fs::create_dir_all(root.join(".hidden")).unwrap();
    fs::write(root.join(".hidden/no.txt"), "x").unwrap();
    fs::create_dir_all(root.join("node_modules/pkg")).unwrap();
    fs::write(root.join("node_modules/pkg/no.js"), "x").unwrap();
    fs::write(root.join("skip.tmp"), "x").unwrap();
    let events = crawl(&folder(root, &["*.tmp"]));
    let ids: Vec<_> = events
        .iter()
        .filter_map(|e| {
            if let SourceEvent::Upsert { external_id, .. } = e {
                Some(external_id.as_str())
            } else {
                None
            }
        })
        .collect();
    assert_eq!(ids.len(), 2);
    assert!(ids.contains(&root.join("keep.txt").to_string_lossy().as_ref()));
    assert!(ids.contains(
        &root
            .join("nested")
            .join("also.md")
            .to_string_lossy()
            .as_ref()
    ));
}
