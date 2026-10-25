//! Configuration and filesystem locations.
//!
//! Resolves the app data directory and its subfolders (database, search
//! index, logs, model cache) and exposes them through a single `AppPaths`
//! struct so no other module builds paths by hand.

use crate::error::Result;
use std::path::{Path, PathBuf};

/// Every location the app reads or writes lives under one root directory.
/// Other modules must ask this struct for paths and never build their own.
#[derive(Debug, Clone)]
pub struct AppPaths {
    root: PathBuf,
}

impl AppPaths {
    /// Pure: does not touch the disk.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Creates the root and all subfolders. Safe to call on every startup.
    pub fn ensure_dirs(&self) -> Result<()> {
        for dir in [
            self.root(),
            &self.db_dir(),
            &self.index_dir(),
            &self.logs_dir(),
            &self.models_dir(),
        ] {
            std::fs::create_dir_all(dir)?;
        }
        Ok(())
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn db_dir(&self) -> PathBuf {
        self.root.join("db")
    }
    pub fn db_file(&self) -> PathBuf {
        self.db_dir().join("app.db")
    }
    pub fn index_dir(&self) -> PathBuf {
        self.root.join("index")
    }
    pub fn logs_dir(&self) -> PathBuf {
        self.root.join("logs")
    }
    pub fn models_dir(&self) -> PathBuf {
        self.root.join("models")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ensure_dirs_creates_all_subfolders() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = AppPaths::new(tmp.path());
        paths.ensure_dirs().unwrap();

        assert!(paths.db_dir().is_dir());
        assert!(paths.index_dir().is_dir());
        assert!(paths.logs_dir().is_dir());
        assert!(paths.models_dir().is_dir());
    }

    #[test]
    fn ensure_dirs_is_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = AppPaths::new(tmp.path());
        paths.ensure_dirs().unwrap();
        paths.ensure_dirs().unwrap(); // second call must not fail
    }

    #[test]
    fn db_file_lives_inside_db_dir() {
        let paths = AppPaths::new("/some/root");
        assert!(paths.db_file().starts_with(paths.db_dir()));
    }
}