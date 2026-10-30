use native_desktop_search_lib::extract::{Extractor, TextExtractor};
use std::path::Path;

#[test]
fn unavailable_file_returns_an_error() {
    let missing = Path::new("this-file-should-not-exist-for-native-desktop-search-test.txt");
    assert!(TextExtractor.extract(missing).is_err());
    let directory = tempfile::tempdir().unwrap();
    assert!(TextExtractor.extract(directory.path()).is_err());
}
