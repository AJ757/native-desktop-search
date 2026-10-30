use native_desktop_search_lib::extract::{Extractor, TextExtractor};
use std::fs;

#[test]
fn invalid_office_documents_return_errors_without_panicking() {
    let tmp = tempfile::tempdir().unwrap();
    for filename in ["broken.docx", "broken.pptx", "broken.xlsx"] {
        let path = tmp.path().join(filename);
        fs::write(&path, b"not a valid office archive").unwrap();
        assert!(TextExtractor.supports(&path));
        assert!(
            TextExtractor.extract(&path).is_err(),
            "{filename} should fail cleanly"
        );
    }
}
