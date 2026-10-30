use crate::support;
use native_desktop_search_lib::sources::SourceEvent;
use std::fs;
use support::*;

#[test]
fn nested_exact_and_suffix_exclusions_apply_to_every_depth() {
    let tmp = temp();
    let root = tmp.path();
    fs::create_dir_all(root.join("keep/nested/private/deeper")).unwrap();
    fs::create_dir_all(root.join("keep/nested/public")).unwrap();
    fs::write(root.join("keep/nested/private/deeper/secret.txt"), "secret").unwrap();
    fs::write(root.join("keep/nested/public/backup.bak"), "backup").unwrap();
    fs::write(root.join("keep/nested/public/visible.txt"), "visible").unwrap();
    let events = crawl(&folder(root, &["private", "*.bak"]));
    let names: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            SourceEvent::Upsert { external_id, .. } => Some(external_id.as_str()),
            SourceEvent::Delete { .. } => None,
        })
        .collect();
    assert_eq!(names.len(), 1);
    assert!(names[0].ends_with("visible.txt"));
}
