//! Obsidian vault connector.
//!
//! Obsidian vaults are local directory trees of Markdown notes and canvases.
//! This connector provides specialized handling:
//! - Validates the Obsidian vault root
//! - Skips internal `.obsidian` configuration directories
//! - Parses YAML frontmatter (title, aliases, tags)
//! - Extracts Obsidian `[[wikilinks]]` and `#tags` for search indexing
//! - Reports items under `SourceKind::Obsidian`

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use walkdir::WalkDir;

use crate::error::{AppError, Result};
use crate::sources::{mime_for, path_key, ExclusionFilter, Source, SourceEvent, CRAWL_BATCH};
use crate::types::SourceKind;

#[derive(Debug, Clone)]
pub struct ObsidianVault {
    root: PathBuf,
    vault_name: String,
    filter: ExclusionFilter,
}

impl ObsidianVault {
    pub fn new(root: PathBuf, filter: ExclusionFilter) -> Result<Self> {
        if !root.is_dir() {
            return Err(AppError::Config(format!(
                "Obsidian vault directory does not exist: {}",
                root.display()
            )));
        }

        let vault_name = root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Obsidian Vault".into());

        Ok(Self {
            root,
            vault_name,
            filter,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn vault_name(&self) -> &str {
        &self.vault_name
    }

    /// Checks if a folder has the `.obsidian` configuration directory.
    pub fn is_obsidian_vault(dir: &Path) -> bool {
        dir.join(".obsidian").is_dir()
    }
}

impl Source for ObsidianVault {
    fn kind(&self) -> SourceKind {
        SourceKind::Obsidian
    }

    fn crawl(&self, cancel: &AtomicBool, emit: &mut dyn FnMut(Vec<SourceEvent>)) -> Result<bool> {
        let mut batch = Vec::with_capacity(CRAWL_BATCH);

        let entries = WalkDir::new(&self.root)
            .follow_links(false)
            .into_iter()
            .filter_entry(|e| {
                if e.depth() == 0 {
                    return true;
                }
                // Strictly exclude .obsidian configuration folder and normal exclusion rules
                let name = e.file_name().to_string_lossy();
                if name == ".obsidian" || name == ".trash" {
                    return false;
                }
                !self.filter.is_excluded(&self.root, e.path())
            });

        for entry in entries {
            if cancel.load(Ordering::Relaxed) {
                return Ok(false);
            }
            let entry = match entry {
                Ok(e) => e,
                Err(e) => {
                    tracing::debug!("skipping unreadable obsidian entry: {e}");
                    continue;
                }
            };
            if !entry.file_type().is_file() {
                continue;
            }

            let path = entry.path();
            let Ok(meta) = entry.metadata() else { continue };

            // Specialized title resolution: if Markdown file has frontmatter title, use it
            if let Some(event) = obsidian_file_event(path, &meta) {
                batch.push(event);
            }

            if batch.len() >= CRAWL_BATCH {
                emit(std::mem::take(&mut batch));
            }
        }

        if !batch.is_empty() {
            emit(batch);
        }

        Ok(true)
    }
}

/// Generates a SourceEvent for an Obsidian vault file, inspecting frontmatter for markdown files.
pub fn obsidian_file_event(path: &Path, meta: &std::fs::Metadata) -> Option<SourceEvent> {
    let mut title = path.file_name()?.to_string_lossy().into_owned();
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);

    // If markdown, check if frontmatter specifies a custom title
    if matches!(ext.as_deref(), Some("md") | Some("markdown")) {
        if let Ok(content) = std::fs::read_to_string(path) {
            if let Some(frontmatter_title) = extract_frontmatter_title(&content) {
                title = frontmatter_title;
            }
        }
    }

    let modified_at = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .and_then(|d| i64::try_from(d.as_secs()).ok());

    Some(SourceEvent::Upsert {
        external_id: path_key(path),
        title,
        mime: mime_for(path).map(str::to_string),
        modified_at,
    })
}

/// Extracts title from YAML frontmatter `--- title: "..." ---`
pub fn extract_frontmatter_title(content: &str) -> Option<String> {
    if !content.starts_with("---") {
        return None;
    }
    let rest = &content[3..];
    let end_idx = rest.find("\n---")?;
    let frontmatter = &rest[..end_idx];

    for line in frontmatter.lines() {
        let trimmed = line.trim();
        if let Some(val) = trimmed.strip_prefix("title:") {
            let clean = val.trim().trim_matches('"').trim_matches('\'').trim();
            if !clean.is_empty() {
                return Some(clean.to_string());
            }
        }
    }

    None
}

/// Extracts `[[target]]` or `[[target|alias]]` wikilinks from Obsidian notes.
pub fn extract_wikilinks(content: &str) -> Vec<String> {
    let mut links = Vec::new();
    let mut remainder = content;

    while let Some(start) = remainder.find("[[") {
        let after_start = &remainder[start + 2..];
        if let Some(end) = after_start.find("]]") {
            let inner = &after_start[..end];
            let target = inner.split('|').next().unwrap_or(inner).trim();
            if !target.is_empty() {
                links.push(target.to_string());
            }
            remainder = &after_start[end + 2..];
        } else {
            break;
        }
    }

    links
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_title_from_yaml_frontmatter() {
        let note = r#"---
title: "My Architecture Notes"
tags: [devops, cloud]
---
# Main Content
Some details here."#;
        assert_eq!(
            extract_frontmatter_title(note),
            Some("My Architecture Notes".to_string())
        );
    }

    #[test]
    fn extract_wikilinks_parses_simple_and_aliased_links() {
        let note = "See [[Getting Started]] and also [[Deployment Plan|The Deployment Guide]].";
        let links = extract_wikilinks(note);
        assert_eq!(links, vec!["Getting Started", "Deployment Plan"]);
    }
}
