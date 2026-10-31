use native_desktop_search_lib::extract::{Extractor, TextExtractor};
use std::fs;

#[test]
fn large_text_document_is_extracted_without_truncation() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("large.txt");
    let content = format!("{}END_MARKER", "document body ".repeat(400_000));
    fs::write(&path, &content).unwrap();
    let extracted = TextExtractor.extract(&path).unwrap();
    assert_eq!(extracted.text.len(), content.len());
    assert!(extracted.text.ends_with("END_MARKER"));
}
