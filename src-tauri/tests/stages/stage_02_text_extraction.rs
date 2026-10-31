use native_desktop_search_lib::extract::{Extractor, TextExtractor};
use std::fs;

#[test]
fn extracts_plain_text_and_strips_markup() {
    let tmp = tempfile::tempdir().unwrap();
    let plain = tmp.path().join("note.txt");
    fs::write(&plain, "searchable note").unwrap();
    let extracted = TextExtractor.extract(&plain).unwrap();
    assert_eq!(extracted.title, "note.txt");
    assert_eq!(extracted.text, "searchable note");
    let html = tmp.path().join("page.html");
    fs::write(&html, "<h1>Hello &amp; world</h1><script>hidden()</script>").unwrap();
    let text = TextExtractor.extract(&html).unwrap().text;
    assert!(text.contains("Hello & world"));
    assert!(!text.contains("hidden"));
    assert!(!TextExtractor.supports(&tmp.path().join("image.exe")));
}
