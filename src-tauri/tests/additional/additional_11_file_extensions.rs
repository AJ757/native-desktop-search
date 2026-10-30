use native_desktop_search_lib::extract::{Extractor, TextExtractor};
use std::{fs, io::Write};
use zip::{write::FileOptions, ZipWriter};

fn write_zip(path: &std::path::Path, entries: &[(&str, &str)]) {
    let file = fs::File::create(path).unwrap();
    let mut archive = ZipWriter::new(file);
    for (name, content) in entries {
        archive.start_file(*name, FileOptions::default()).unwrap();
        archive.write_all(content.as_bytes()).unwrap();
    }
    archive.finish().unwrap();
}

#[test]
fn recognizes_all_supported_extensions() {
    let tmp = tempfile::tempdir().unwrap();
    for extension in [
        "txt", "md", "markdown", "csv", "json", "log", "xml", "html", "htm", "pdf", "docx", "pptx",
        "xlsx", "xls", "rtf",
    ] {
        let path = tmp.path().join(format!("sample.{extension}"));
        assert!(
            TextExtractor.supports(&path),
            "expected .{extension} to be supported"
        );
    }
}

#[test]
fn extracts_text_from_plain_markup_and_office_formats() {
    let tmp = tempfile::tempdir().unwrap();
    for (filename, content, expected) in [
        ("sample.txt", "plain needle", "plain needle"),
        ("sample.md", "# markdown needle", "# markdown needle"),
        ("sample.markdown", "markdown note", "markdown note"),
        ("sample.csv", "name,value\nneedle,42", "needle,42"),
        ("sample.json", r#"{"word":"needle"}"#, "needle"),
        ("sample.log", "log needle", "log needle"),
        (
            "sample.xml",
            "<item>xml needle</item>",
            "<item>xml needle</item>",
        ),
    ] {
        let path = tmp.path().join(filename);
        fs::write(&path, content).unwrap();
        let extracted = TextExtractor.extract(&path).unwrap();
        assert!(extracted.text.contains(expected), "failed for {filename}");
    }

    for filename in ["sample.html", "sample.htm"] {
        let path = tmp.path().join(filename);
        fs::write(&path, "<p>html needle</p><script>discarded</script>").unwrap();
        let text = TextExtractor.extract(&path).unwrap().text;
        assert!(text.contains("html needle"));
        assert!(!text.contains("discarded"));
    }

    let rtf = tmp.path().join("sample.rtf");
    fs::write(&rtf, r#"{\rtf1\ansi Rich text needle}"#).unwrap();
    assert!(TextExtractor
        .extract(&rtf)
        .unwrap()
        .text
        .contains("Rich text needle"));

    let docx = tmp.path().join("sample.docx");
    write_zip(
        &docx,
        &[(
            "word/document.xml",
            "<document><p>docx needle</p></document>",
        )],
    );
    assert!(TextExtractor
        .extract(&docx)
        .unwrap()
        .text
        .contains("docx needle"));

    let pptx = tmp.path().join("sample.pptx");
    write_zip(
        &pptx,
        &[("ppt/slides/slide1.xml", "<slide><p>pptx needle</p></slide>")],
    );
    assert!(TextExtractor
        .extract(&pptx)
        .unwrap()
        .text
        .contains("pptx needle"));
}
