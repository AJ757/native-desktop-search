//! Data sources: local folders and cloud connectors.
//!
//! Every source reports what it holds as `SourceEvent`s (upsert or delete),
//! so the sync worker stores all of them the same way. Local folders are
//! implemented here. Cloud connectors join in the Connect stage and will
//! extend the `Source` trait with cursors, content fetching and open targets.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::UNIX_EPOCH;

use walkdir::WalkDir;

use crate::error::{AppError, Result};
use crate::types::SourceKind;

/// How many events a crawl sends to the sync worker at once.
/// Each batch is written in one transaction.
pub const CRAWL_BATCH: usize = 500;

/// Folder and file names skipped in every local source, on top of hidden
/// entries (names starting with a dot).
const DEFAULT_EXCLUDED_NAMES: &[&str] = &[
    "node_modules",
    "target",
    "__pycache__",
    "$RECYCLE.BIN",
    "System Volume Information",
];

/// One change reported by a source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceEvent {
    Upsert {
        external_id: String,
        title: String,
        mime: Option<String>,
        modified_at: Option<i64>,
    },
    /// The item is gone. For local folders this also covers everything below
    /// the path, so deleting or moving a folder is one event.
    Delete { external_id: String },
}

pub trait Source: Send + Sync {
    fn kind(&self) -> SourceKind;

    /// Reports every item the source holds, in batches, through `emit`.
    /// Returns `Ok(false)` if `cancel` was set before the crawl finished.
    fn crawl(&self, cancel: &AtomicBool, emit: &mut dyn FnMut(Vec<SourceEvent>)) -> Result<bool>;
}

/// True for source kinds that are plain folders on this machine.
pub fn is_folder_kind(kind: SourceKind) -> bool {
    matches!(kind, SourceKind::Local | SourceKind::Obsidian)
}

/// Reads the folder path from a source's JSON config (`{"path": "..."}`)
/// and returns it as an absolute path to an existing directory.
pub fn folder_root_from_config(config: &serde_json::Value) -> Result<PathBuf> {
    let raw = config
        .get("path")
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| AppError::Config("folder source needs a \"path\"".into()))?;
    // std::path::absolute keeps Windows paths in their normal form
    // (canonicalize would turn them into \\?\ paths).
    let root = std::path::absolute(raw)?;
    if !root.is_dir() {
        return Err(AppError::Config(format!(
            "folder does not exist: {}",
            root.display()
        )));
    }
    Ok(root)
}

/// Decides which paths inside a folder source are skipped.
#[derive(Debug, Clone, Default)]
pub struct ExclusionFilter {
    /// Exact file or folder names, e.g. `build`.
    names: Vec<String>,
    /// Name endings from `*.ext` patterns, e.g. `.tmp`.
    suffixes: Vec<String>,
    /// Whole directory trees, e.g. the app's own data folder.
    roots: Vec<PathBuf>,
}

impl ExclusionFilter {
    /// `patterns` come from the exclusions table. A pattern is either an
    /// exact name (`build`) or a name ending (`*.tmp`). `roots` are whole
    /// directories to skip.
    pub fn new(patterns: &[String], roots: Vec<PathBuf>) -> Self {
        let mut names: Vec<String> = DEFAULT_EXCLUDED_NAMES
            .iter()
            .map(|s| s.to_string())
            .collect();
        let mut suffixes = Vec::new();
        for p in patterns.iter().map(|p| p.trim()).filter(|p| !p.is_empty()) {
            match p.strip_prefix('*') {
                Some(ending) => suffixes.push(ending.to_string()),
                None => names.push(p.to_string()),
            }
        }
        Self {
            names,
            suffixes,
            roots,
        }
    }

    /// True if `path` (somewhere below `source_root`) must be skipped.
    /// Only the part below the source root is checked, so a source whose own
    /// folder is hidden (like `~/.notes`) still works.
    pub fn is_excluded(&self, source_root: &Path, path: &Path) -> bool {
        if self.roots.iter().any(|r| path.starts_with(r)) {
            return true;
        }
        let Ok(relative) = path.strip_prefix(source_root) else {
            return true; // outside the source
        };
        relative.components().any(|c| {
            let name = c.as_os_str().to_string_lossy();
            name.starts_with('.')
                || self.names.iter().any(|n| *n == name)
                || self.suffixes.iter().any(|s| name.ends_with(s.as_str()))
        })
    }
}

/// A folder on this machine (a plain folder or an Obsidian vault).
#[derive(Debug, Clone)]
pub struct LocalFolder {
    kind: SourceKind,
    root: PathBuf,
    filter: ExclusionFilter,
}

impl LocalFolder {
    pub fn new(kind: SourceKind, root: PathBuf, filter: ExclusionFilter) -> Self {
        Self { kind, root, filter }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Events for one path the file watcher reported.
    ///
    /// The path is looked up on disk instead of trusting the event kind:
    /// a file that exists is upserted, a path that is gone is deleted
    /// (with everything below it). A directory that appeared is walked only
    /// when `walk_new_dirs` is set, so a plain "folder modified" event does
    /// not rescan a whole tree.
    pub fn changes_at(&self, path: &Path, walk_new_dirs: bool) -> Vec<SourceEvent> {
        if path == self.root || self.filter.is_excluded(&self.root, path) {
            return Vec::new();
        }
        match std::fs::symlink_metadata(path) {
            Ok(meta) if meta.is_file() => file_event(path, &meta).into_iter().collect(),
            Ok(meta) if meta.is_dir() && walk_new_dirs => {
                let mut events = Vec::new();
                let never = AtomicBool::new(false);
                self.walk(path, &never, &mut |batch| events.extend(batch));
                events
            }
            Ok(_) => Vec::new(), // symlinks, sockets, or a directory we don't rescan
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => vec![SourceEvent::Delete {
                external_id: path_key(path),
            }],
            Err(e) => {
                tracing::debug!(path = %path.display(), "skipping unreadable path: {e}");
                Vec::new()
            }
        }
    }

    /// Walks `dir` (inside the root) and emits upserts for every file.
    /// Returns false if cancelled.
    fn walk(
        &self,
        dir: &Path,
        cancel: &AtomicBool,
        emit: &mut dyn FnMut(Vec<SourceEvent>),
    ) -> bool {
        let mut batch = Vec::with_capacity(CRAWL_BATCH);
        let entries = WalkDir::new(dir)
            .follow_links(false)
            .into_iter()
            .filter_entry(|e| e.depth() == 0 || !self.filter.is_excluded(&self.root, e.path()));
        for entry in entries {
            if cancel.load(Ordering::Relaxed) {
                return false;
            }
            let entry = match entry {
                Ok(e) => e,
                Err(e) => {
                    tracing::debug!("skipping unreadable entry: {e}");
                    continue;
                }
            };
            if !entry.file_type().is_file() {
                continue;
            }
            let Ok(meta) = entry.metadata() else { continue };
            batch.extend(file_event(entry.path(), &meta));
            if batch.len() >= CRAWL_BATCH {
                emit(std::mem::take(&mut batch));
            }
        }
        if !batch.is_empty() {
            emit(batch);
        }
        true
    }
}

impl Source for LocalFolder {
    fn kind(&self) -> SourceKind {
        self.kind
    }

    fn crawl(&self, cancel: &AtomicBool, emit: &mut dyn FnMut(Vec<SourceEvent>)) -> Result<bool> {
        if !self.root.is_dir() {
            return Err(AppError::Config(format!(
                "folder is missing: {}",
                self.root.display()
            )));
        }
        Ok(self.walk(&self.root, cancel, emit))
    }
}

/// The external id of a local file: its absolute path.
pub fn path_key(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn file_event(path: &Path, meta: &std::fs::Metadata) -> Option<SourceEvent> {
    let title = path.file_name()?.to_string_lossy().into_owned();
    let modified_at = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .and_then(|d| i64::try_from(d.as_secs()).ok());
    Some(SourceEvent::Upsert {
        external_id: path_key(path),
        title,
        mime: mime_for(path).map(str::to_string),
        modified_at,
    })
}

/// MIME type from the file extension, for the formats search cares about.
pub fn mime_for(path: &Path) -> Option<&'static str> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    Some(match ext.as_str() {
        "txt" | "log" => "text/plain",
        "md" | "markdown" => "text/markdown",
        "csv" => "text/csv",
        "html" | "htm" => "text/html",
        "json" => "application/json",
        "rtf" => "application/rtf",
        "pdf" => "application/pdf",
        "doc" => "application/msword",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xls" => "application/vnd.ms-excel",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "ppt" => "application/vnd.ms-powerpoint",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "odt" => "application/vnd.oasis.opendocument.text",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn folder(root: &Path, patterns: &[&str]) -> LocalFolder {
        let patterns: Vec<String> = patterns.iter().map(|s| s.to_string()).collect();
        LocalFolder::new(
            SourceKind::Local,
            root.to_path_buf(),
            ExclusionFilter::new(&patterns, Vec::new()),
        )
    }

    fn crawl_ids(f: &LocalFolder) -> Vec<String> {
        let mut ids = Vec::new();
        let done = f
            .crawl(&AtomicBool::new(false), &mut |batch| {
                for ev in batch {
                    if let SourceEvent::Upsert { external_id, .. } = ev {
                        ids.push(external_id);
                    }
                }
            })
            .unwrap();
        assert!(done);
        ids.sort();
        ids
    }

    fn touch(path: &Path) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, "x").unwrap();
    }

    #[test]
    fn crawl_finds_nested_files_and_skips_excluded_ones() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        touch(&root.join("a.md"));
        touch(&root.join("sub/b.txt"));
        touch(&root.join(".git/config"));
        touch(&root.join("node_modules/pkg/index.js"));
        touch(&root.join("build/out.txt"));
        touch(&root.join("scratch.tmp"));

        let ids = crawl_ids(&folder(root, &["build", "*.tmp"]));

assert_eq!(
    ids,
    vec![
        path_key(&root.join("a.md")),
        path_key(&root.join("sub").join("b.txt")),
    ]
);
    }

    #[test]
    fn crawl_reports_title_mime_and_mtime() {
        let tmp = tempfile::tempdir().unwrap();
        touch(&tmp.path().join("Notes.MD"));
        let mut events = Vec::new();
        folder(tmp.path(), &[])
            .crawl(&AtomicBool::new(false), &mut |b| events.extend(b))
            .unwrap();
        match &events[..] {
            [SourceEvent::Upsert {
                title,
                mime,
                modified_at,
                ..
            }] => {
                assert_eq!(title, "Notes.MD");
                assert_eq!(mime.as_deref(), Some("text/markdown"));
                assert!(modified_at.is_some());
            }
            other => panic!("unexpected events: {other:?}"),
        }
    }

    #[test]
    fn crawl_sends_large_folders_in_batches() {
        let tmp = tempfile::tempdir().unwrap();
        for i in 0..(CRAWL_BATCH + 3) {
            touch(&tmp.path().join(format!("f{i}.txt")));
        }
        let mut sizes = Vec::new();
        folder(tmp.path(), &[])
            .crawl(&AtomicBool::new(false), &mut |b| sizes.push(b.len()))
            .unwrap();
        assert_eq!(sizes, vec![CRAWL_BATCH, 3]);
    }

    #[test]
    fn cancelled_crawl_reports_not_done() {
        let tmp = tempfile::tempdir().unwrap();
        touch(&tmp.path().join("a.txt"));
        let done = folder(tmp.path(), &[])
            .crawl(&AtomicBool::new(true), &mut |_| {})
            .unwrap();
        assert!(!done);
    }

    #[test]
    fn hidden_source_root_still_works() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join(".notes");
        touch(&root.join("a.txt"));
        assert_eq!(crawl_ids(&folder(&root, &[])).len(), 1);
    }

    #[test]
    fn excluded_roots_skip_the_app_data_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let app_data = tmp.path().join("appdata");
        touch(&app_data.join("logs/app.log"));
        touch(&tmp.path().join("keep.txt"));
        let f = LocalFolder::new(
            SourceKind::Local,
            tmp.path().to_path_buf(),
            ExclusionFilter::new(&[], vec![app_data]),
        );
        assert_eq!(crawl_ids(&f), vec![path_key(&tmp.path().join("keep.txt"))]);
    }

    #[test]
    fn changes_at_upserts_existing_files_and_deletes_missing_paths() {
        let tmp = tempfile::tempdir().unwrap();
        let f = folder(tmp.path(), &[]);
        let file = tmp.path().join("a.txt");
        touch(&file);
        assert!(matches!(
            f.changes_at(&file, false)[..],
            [SourceEvent::Upsert { .. }]
        ));

        fs::remove_file(&file).unwrap();
        assert_eq!(
            f.changes_at(&file, false),
            vec![SourceEvent::Delete {
                external_id: path_key(&file)
            }]
        );
    }

    #[test]
    fn changes_at_walks_new_directories_only_when_asked() {
        let tmp = tempfile::tempdir().unwrap();
        let f = folder(tmp.path(), &[]);
        let dir = tmp.path().join("moved-in");
        touch(&dir.join("a.txt"));
        touch(&dir.join("deep/b.txt"));
        assert_eq!(f.changes_at(&dir, true).len(), 2);
        assert!(f.changes_at(&dir, false).is_empty());
    }

    #[test]
    fn changes_at_ignores_excluded_paths_and_the_root() {
        let tmp = tempfile::tempdir().unwrap();
        let f = folder(tmp.path(), &[]);
        let hidden = tmp.path().join(".cache/x.txt");
        touch(&hidden);
        assert!(f.changes_at(&hidden, true).is_empty());
        assert!(f.changes_at(tmp.path(), true).is_empty());
    }

    #[test]
    fn config_path_must_be_an_existing_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let ok = serde_json::json!({ "path": tmp.path() });
        assert_eq!(folder_root_from_config(&ok).unwrap(), tmp.path());

        let missing = serde_json::json!({ "path": tmp.path().join("nope") });
        assert!(matches!(
            folder_root_from_config(&missing),
            Err(AppError::Config(_))
        ));
        assert!(matches!(
            folder_root_from_config(&serde_json::json!({})),
            Err(AppError::Config(_))
        ));
    }
}
