use serde::{Serialize, Serializer};

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),

    #[error("migration error: {0}")]
    Migration(#[from] rusqlite_migration::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("file watcher error: {0}")]
    Watch(#[from] notify_debouncer_full::notify::Error),

    #[error("Tantivy error: {0}")]
    Tantivy(String),

    #[error("Internal error: {0}")]
    Internal(String),

    #[error("configuration error: {0}")]
    Config(String),

    #[error("not found: {0}")]
    NotFound(String),

    #[error("{0}")]
    Other(String),
}

impl Serialize for AppError {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl From<tantivy::TantivyError> for AppError {
    fn from(err: tantivy::TantivyError) -> Self {
        Self::Tantivy(err.to_string())
    }
}

impl From<tantivy::directory::error::OpenDirectoryError> for AppError {
    fn from(err: tantivy::directory::error::OpenDirectoryError) -> Self {
        Self::Tantivy(err.to_string())
    }
}

impl From<tantivy::query::QueryParserError> for AppError {
    fn from(err: tantivy::query::QueryParserError) -> Self {
        Self::Tantivy(err.to_string())
    }
}

pub type Result<T> = std::result::Result<T, AppError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_as_plain_message() {
        let err = AppError::NotFound("source 7".into());

        assert_eq!(
            serde_json::to_string(&err).unwrap(),
            "\"not found: source 7\""
        );
    }

    #[test]
    fn io_errors_convert_with_question_mark() {
        fn fails() -> Result<()> {
            std::fs::read("definitely/does/not/exist")?;
            Ok(())
        }

        assert!(matches!(fails(), Err(AppError::Io(_))));
    }
}
