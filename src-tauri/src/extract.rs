use std::fs;
use std::io::{self, Read};
use std::path::Path;

use calamine::{open_workbook_auto, Reader};
use regex::Regex;
use zip::ZipArchive;

use crate::error::Result;

#[derive(Debug, Clone)]
pub struct ExtractedDocument {
    pub title: String,
    pub text: String,
}

pub trait Extractor: Send + Sync {
    fn supports(&self, path: &Path) -> bool;
    fn extract(&self, path: &Path) -> Result<ExtractedDocument>;
}

pub struct TextExtractor;

impl Extractor for TextExtractor {
    fn supports(&self, path: &Path) -> bool {
        matches!(
            path.extension()
                .and_then(|ext| ext.to_str())
                .map(str::to_ascii_lowercase)
                .as_deref(),
            Some(
                "txt"
                    | "md"
                    | "markdown"
                    | "csv"
                    | "json"
                    | "log"
                    | "xml"
                    | "html"
                    | "htm"
                    | "pdf"
                    | "docx"
                    | "pptx"
                    | "xlsx"
                    | "xls"
                    | "rtf"
            )
        )
    }

    fn extract(&self, path: &Path) -> Result<ExtractedDocument> {
        let extension = path
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();

        let text = match extension.as_str() {
            "pdf" => {
                let pdf_path = path.to_path_buf();
                std::panic::catch_unwind(|| pdf_extract::extract_text(&pdf_path))
                    .map_err(|_| io::Error::other("PDF extraction panicked"))?
                    .map_err(|e| io::Error::other(format!("PDF extraction failed: {e}")))?
            }

            "docx" => extract_office_xml(path, "word/document.xml")?,

            "pptx" => extract_presentation(path)?,

            "xlsx" | "xls" => extract_spreadsheet(path)?,

            "html" | "htm" => {
                let content = fs::read_to_string(path)?;
                strip_markup(&content)
            }

            "rtf" => {
                let content = fs::read_to_string(path)?;
                strip_rtf(&content)
            }

            _ => fs::read_to_string(path)?,
        };

        let title = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("Untitled")
            .to_string();

        Ok(ExtractedDocument { title, text })
    }
}

pub fn extract(path: &Path) -> Result<ExtractedDocument> {
    TextExtractor.extract(path)
}

fn extract_office_xml(path: &Path, member: &str) -> Result<String> {
    let file = fs::File::open(path)?;
    let mut archive = ZipArchive::new(file).map_err(|e| io::Error::other(e.to_string()))?;

    let mut xml = String::new();
    archive
        .by_name(member)
        .map_err(|e| io::Error::other(e.to_string()))?
        .read_to_string(&mut xml)?;

    Ok(xml_text(&xml))
}

fn extract_presentation(path: &Path) -> Result<String> {
    let file = fs::File::open(path)?;
    let mut archive = ZipArchive::new(file).map_err(|e| io::Error::other(e.to_string()))?;

    let mut output = String::new();

    for i in 0..archive.len() {
        let name = archive
            .by_index(i)
            .map_err(|e| io::Error::other(e.to_string()))?
            .name()
            .to_string();

        if name.starts_with("ppt/slides/slide") && name.ends_with(".xml") {
            let mut xml = String::new();
            archive
                .by_name(&name)
                .map_err(|e| io::Error::other(e.to_string()))?
                .read_to_string(&mut xml)?;

            output.push_str(&xml_text(&xml));
            output.push('\n');
        }
    }

    Ok(output)
}

fn extract_spreadsheet(path: &Path) -> Result<String> {
    let mut workbook = open_workbook_auto(path).map_err(|e| io::Error::other(e.to_string()))?;

    let mut output = String::new();

    for sheet_name in workbook.sheet_names().to_owned() {
        if let Ok(range) = workbook.worksheet_range(&sheet_name) {
            for row in range.rows() {
                for cell in row {
                    output.push_str(&cell.to_string());
                    output.push(' ');
                }
                output.push('\n');
            }
        }
    }

    Ok(output)
}

fn xml_text(xml: &str) -> String {
    let re = Regex::new(r"<[^>]+>").unwrap();
    let text = re.replace_all(xml, " ");
    decode_entities(&text)
}

fn strip_markup(html: &str) -> String {
    let without_scripts =
        Regex::new(r"(?is)<script\b[^>]*>.*?</script\s*>|<style\b[^>]*>.*?</style\s*>")
            .unwrap()
            .replace_all(html, " ");

    let text = Regex::new(r"(?s)<[^>]+>")
        .unwrap()
        .replace_all(&without_scripts, " ");

    decode_entities(&text)
}

fn strip_rtf(rtf: &str) -> String {
    let without_controls = Regex::new(r"\\[a-zA-Z]+-?\d* ?|\\[^a-zA-Z]")
        .unwrap()
        .replace_all(rtf, " ");

    Regex::new(r"[{}]")
        .unwrap()
        .replace_all(&without_controls, " ")
        .to_string()
}

fn decode_entities(input: &str) -> String {
    input
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&nbsp;", " ")
}
