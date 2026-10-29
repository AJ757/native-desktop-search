//! SQLite access: connection setup, migrations and queries.
//!
//! SQLite is the source of truth. The Tantivy index and the vector store
//! must always be rebuildable from what is stored here.

use std::path::Path;
use std::time::Duration;

use rusqlite::{params, Connection, Row};
use rusqlite_migration::{Migrations, M};

use serde::de::DeserializeOwned;

use crate::types::{AddSourceRequest, Source, SourceStatus};

use crate::error::{AppError, Result};

fn migrations() -> Migrations<'static> {
    Migrations::new(vec![
        M::up(include_str!("../migrations/001_init.sql")),
        M::up(include_str!("../migrations/002_seen_at.sql")),
        M::up(include_str!("../migrations/003_embeddings.sql")),
    ])
}

fn configure(conn: &Connection) -> Result<()> {
    conn.busy_timeout(Duration::from_secs(5))?;
    let mode: String =
        conn.pragma_update_and_check(None, "journal_mode", "WAL", |row| row.get(0))?;
    tracing::debug!(journal_mode = %mode, "sqlite configured");
    conn.pragma_update(None, "foreign_keys", true)?;
    Ok(())
}

fn prepare(mut conn: Connection) -> Result<Connection> {
    configure(&conn)?; // pragmas first, outside any migration transaction
    migrations().to_latest(&mut conn)?;
    Ok(conn)
}

pub fn open(path: &Path) -> Result<Connection> {
    prepare(Connection::open(path)?)
}

pub fn open_in_memory() -> Result<Connection> {
    prepare(Connection::open_in_memory()?)
}

type RawSource = (i64, String, String, String);

fn raw_source(row: &Row) -> rusqlite::Result<RawSource> {
    Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
}

/// Parse "google_drive" etc. back into an enum using its serde form.
fn parse_enum<T: DeserializeOwned>(s: String) -> Result<T> {
    Ok(serde_json::from_value(serde_json::Value::String(s))?)
}

fn into_source((id, kind, config, status): RawSource) -> Result<Source> {
    Ok(Source {
        id,
        kind: parse_enum(kind)?,
        config: serde_json::from_str(&config)?,
        status: parse_enum(status)?,
    })
}

pub fn list_sources(conn: &Connection) -> Result<Vec<Source>> {
    let mut stmt = conn.prepare("SELECT id, kind, config, status FROM sources ORDER BY id")?;
    let raws = stmt
        .query_map([], raw_source)?
        .collect::<std::result::Result<Vec<RawSource>, _>>()?;
    raws.into_iter().map(into_source).collect()
}

pub fn insert_source(conn: &Connection, req: &AddSourceRequest) -> Result<Source> {
    conn.execute(
        "INSERT INTO sources (kind, config) VALUES (?1, ?2)",
        params![req.kind.as_str(), req.config.to_string()],
    )?;
    let id = conn.last_insert_rowid();
    let raw = conn.query_row(
        "SELECT id, kind, config, status FROM sources WHERE id = ?1",
        [id],
        raw_source,
    )?;
    into_source(raw)
}

pub fn get_source(conn: &Connection, id: i64) -> Result<Source> {
    let raw = conn
        .query_row(
            "SELECT id, kind, config, status FROM sources WHERE id = ?1",
            [id],
            raw_source,
        )
        .map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => AppError::NotFound(format!("source {id}")),
            other => other.into(),
        })?;
    into_source(raw)
}

/// Deletes a source. Its documents, chunks and sync state go with it (cascade).
pub fn delete_source(conn: &Connection, id: i64) -> Result<()> {
    let n = conn.execute("DELETE FROM sources WHERE id = ?1", [id])?;
    if n == 0 {
        return Err(AppError::NotFound(format!("source {id}")));
    }
    Ok(())
}

pub fn set_source_status(conn: &Connection, id: i64, status: SourceStatus) -> Result<()> {
    conn.execute(
        "UPDATE sources SET status = ?2 WHERE id = ?1",
        params![id, status.as_str()],
    )?;
    Ok(())
}

/// Exclusion patterns for one source, including the global ones (source_id NULL).
pub fn list_exclusion_patterns(conn: &Connection, source_id: i64) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT pattern FROM exclusions WHERE source_id IS NULL OR source_id = ?1 ORDER BY id",
    )?;
    let patterns = stmt
        .query_map([source_id], |r| r.get(0))?
        .collect::<std::result::Result<Vec<String>, _>>()?;
    Ok(patterns)
}

/// What a source knows about one item, ready to store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentUpsert<'a> {
    pub external_id: &'a str,
    pub title: &'a str,
    pub mime: Option<&'a str>,
    pub modified_at: Option<i64>,
}

/// Inserts or refreshes a document and stamps it as seen at `seen_at`.
///
/// If the document is new, was deleted, or its modification time changed,
/// `indexed_at` and `content_hash` are cleared so the indexer picks it up
/// again. An unchanged document keeps them.
pub fn upsert_document(
    conn: &Connection,
    source_id: i64,
    doc: &DocumentUpsert,
    seen_at: i64,
) -> Result<()> {
    // In an UPDATE, SQLite evaluates every right-hand side against the old row,
    // so the CASE expressions compare the old modified_at with the new one.
    conn.execute(
        "INSERT INTO documents (source_id, external_id, title, mime, modified_at, seen_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT (source_id, external_id) DO UPDATE SET
             indexed_at   = CASE WHEN deleted = 0 AND modified_at IS excluded.modified_at
                                 THEN indexed_at ELSE NULL END,
             content_hash = CASE WHEN deleted = 0 AND modified_at IS excluded.modified_at
                                 THEN content_hash ELSE NULL END,
             title        = excluded.title,
             mime         = excluded.mime,
             modified_at  = excluded.modified_at,
             seen_at      = excluded.seen_at,
             deleted      = 0",
        params![
            source_id,
            doc.external_id,
            doc.title,
            doc.mime,
            doc.modified_at,
            seen_at
        ],
    )?;
    Ok(())
}

/// Marks `path` and everything below it as deleted, so removing or renaming
/// a whole folder is a single call. `indexed_at` is kept: a deleted row that
/// is still indexed tells the indexer to remove it from the index.
pub fn mark_deleted_under(
    conn: &Connection,
    source_id: i64,
    path: &str,
    separator: char,
) -> Result<usize> {
    let prefix = format!("{path}{separator}");
    let n = conn.execute(
        "UPDATE documents SET deleted = 1
         WHERE source_id = ?1 AND deleted = 0
           AND (external_id = ?2 OR substr(external_id, 1, length(?3)) = ?3)",
        params![source_id, path, prefix],
    )?;
    Ok(n)
}

/// After a full crawl that began at `crawl_started_at`, marks every live
/// document the crawl did not see as deleted.
pub fn sweep_unseen(conn: &Connection, source_id: i64, crawl_started_at: i64) -> Result<usize> {
    let n = conn.execute(
        "UPDATE documents SET deleted = 1
         WHERE source_id = ?1 AND deleted = 0 AND (seen_at IS NULL OR seen_at < ?2)",
        params![source_id, crawl_started_at],
    )?;
    Ok(n)
}

/// Number of live (not deleted) documents across all sources.
pub fn count_documents(conn: &Connection) -> Result<u32> {
    let n: i64 = conn.query_row(
        "SELECT COUNT(*) FROM documents WHERE deleted = 0",
        [],
        |r| r.get(0),
    )?;
    Ok(u32::try_from(n).unwrap_or(u32::MAX))
}

/// Reads the sync cursor for a source, if any has been saved.
pub fn get_sync_cursor(conn: &Connection, source_id: i64) -> Result<Option<String>> {
    let mut stmt = conn.prepare("SELECT cursor FROM sync_state WHERE source_id = ?1")?;
    let mut rows = stmt.query([source_id])?;
    if let Some(row) = rows.next()? {
        let cursor: Option<String> = row.get(0)?;
        Ok(cursor)
    } else {
        Ok(None)
    }
}

/// Stores or updates the sync cursor and last_synced_at timestamp for a source.
pub fn set_sync_cursor(
    conn: &Connection,
    source_id: i64,
    cursor: &str,
    last_synced_at: i64,
) -> Result<()> {
    conn.execute(
        "INSERT INTO sync_state (source_id, cursor, last_synced_at)
         VALUES (?1, ?2, ?3)
         ON CONFLICT (source_id) DO UPDATE SET
             cursor = excluded.cursor,
             last_synced_at = excluded.last_synced_at",
        params![source_id, cursor, last_synced_at],
    )?;
    Ok(())
}

/// Retrieves the cursor and last_synced_at timestamp for a source.
pub fn get_sync_state(
    conn: &Connection,
    source_id: i64,
) -> Result<Option<(Option<String>, Option<i64>)>> {
    let mut stmt =
        conn.prepare("SELECT cursor, last_synced_at FROM sync_state WHERE source_id = ?1")?;
    let mut rows = stmt.query([source_id])?;
    if let Some(row) = rows.next()? {
        Ok(Some((row.get(0)?, row.get(1)?)))
    } else {
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table_names(conn: &Connection) -> Vec<String> {
        let mut stmt = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table'")
            .unwrap();
        stmt.query_map([], |r| r.get(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect()
    }

    fn user_version(conn: &Connection) -> i64 {
        conn.pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap()
    }

    #[test]
    fn migration_definitions_are_valid() {
        migrations().validate().unwrap();
    }

    #[test]
    fn creates_all_tables() {
        let conn = open_in_memory().unwrap();
        let tables = table_names(&conn);
        for t in [
            "sources",
            "documents",
            "chunks",
            "sync_state",
            "exclusions",
            "settings",
        ] {
            assert!(tables.contains(&t.to_string()), "missing table {t}");
        }
    }

    #[test]
    fn running_migrations_twice_is_a_noop() {
        let mut conn = open_in_memory().unwrap();
        let before = user_version(&conn);
        migrations().to_latest(&mut conn).unwrap();
        assert_eq!(user_version(&conn), before);
        assert_eq!(before, 3);
    }

    #[test]
    fn deleting_a_source_cascades() {
        let conn = open_in_memory().unwrap();
        conn.execute("INSERT INTO sources (id, kind) VALUES (1, 'local')", [])
            .unwrap();
        conn.execute(
            "INSERT INTO documents (source_id, external_id) VALUES (1, 'a.txt')",
            [],
        )
        .unwrap();
        conn.execute("DELETE FROM sources WHERE id = 1", [])
            .unwrap();
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM documents", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0);
    }

    #[test]
    fn document_upsert_key_is_unique() {
        let conn = open_in_memory().unwrap();
        conn.execute("INSERT INTO sources (id, kind) VALUES (1, 'local')", [])
            .unwrap();
        let ins = "INSERT INTO documents (source_id, external_id, title) VALUES (1, 'a.txt', ?1)";
        conn.execute(ins, ["first"]).unwrap();
        assert!(conn.execute(ins, ["dup"]).is_err());

        conn.execute(
            "INSERT INTO documents (source_id, external_id, title) VALUES (1, 'a.txt', 'second')
             ON CONFLICT(source_id, external_id) DO UPDATE SET title = excluded.title",
            [],
        )
        .unwrap();
        let title: String = conn
            .query_row("SELECT title FROM documents", [], |r| r.get(0))
            .unwrap();
        assert_eq!(title, "second");
    }

    #[test]
    fn insert_then_list_sources() {
        let conn = open_in_memory().unwrap();
        assert!(list_sources(&conn).unwrap().is_empty());

        let req = AddSourceRequest {
            kind: crate::types::SourceKind::Local,
            config: serde_json::json!({ "path": "C:/notes" }),
        };
        let added = insert_source(&conn, &req).unwrap();
        assert_eq!(added.kind, crate::types::SourceKind::Local);
        assert_eq!(added.status, crate::types::SourceStatus::Idle);

        let all = list_sources(&conn).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].config["path"], "C:/notes");
    }

    // ---------- documents ----------

    fn conn_with_source() -> Connection {
        let conn = open_in_memory().unwrap();
        conn.execute("INSERT INTO sources (id, kind) VALUES (1, 'local')", [])
            .unwrap();
        conn
    }

    fn doc(external_id: &str, modified_at: i64) -> DocumentUpsert<'_> {
        DocumentUpsert {
            external_id,
            title: "title",
            mime: Some("text/plain"),
            modified_at: Some(modified_at),
        }
    }

    /// (deleted, indexed_at, seen_at) for one document.
    fn doc_state(conn: &Connection, external_id: &str) -> (bool, Option<i64>, Option<i64>) {
        conn.query_row(
            "SELECT deleted, indexed_at, seen_at FROM documents WHERE external_id = ?1",
            [external_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap()
    }

    fn mark_indexed(conn: &Connection, external_id: &str) {
        conn.execute(
            "UPDATE documents SET indexed_at = 50 WHERE external_id = ?1",
            [external_id],
        )
        .unwrap();
    }

    #[test]
    fn upsert_twice_keeps_one_row() {
        let conn = conn_with_source();
        upsert_document(&conn, 1, &doc("/a.txt", 10), 100).unwrap();
        upsert_document(&conn, 1, &doc("/a.txt", 10), 200).unwrap();
        assert_eq!(count_documents(&conn).unwrap(), 1);
        assert_eq!(doc_state(&conn, "/a.txt").2, Some(200));
    }

    #[test]
    fn unchanged_upsert_keeps_indexed_at() {
        let conn = conn_with_source();
        upsert_document(&conn, 1, &doc("/a.txt", 10), 100).unwrap();
        mark_indexed(&conn, "/a.txt");
        upsert_document(&conn, 1, &doc("/a.txt", 10), 200).unwrap();
        assert_eq!(doc_state(&conn, "/a.txt").1, Some(50));
    }

    #[test]
    fn modified_upsert_clears_indexed_at() {
        let conn = conn_with_source();
        upsert_document(&conn, 1, &doc("/a.txt", 10), 100).unwrap();
        mark_indexed(&conn, "/a.txt");
        upsert_document(&conn, 1, &doc("/a.txt", 11), 200).unwrap();
        assert_eq!(doc_state(&conn, "/a.txt").1, None);
    }

    #[test]
    fn upsert_revives_a_deleted_document() {
        let conn = conn_with_source();
        upsert_document(&conn, 1, &doc("/a.txt", 10), 100).unwrap();
        mark_indexed(&conn, "/a.txt");
        mark_deleted_under(&conn, 1, "/a.txt", '/').unwrap();
        upsert_document(&conn, 1, &doc("/a.txt", 10), 200).unwrap();
        let (deleted, indexed_at, _) = doc_state(&conn, "/a.txt");
        assert!(!deleted);
        assert_eq!(indexed_at, None, "revived file must be reindexed");
    }

    #[test]
    fn delete_under_a_folder_covers_children_but_not_siblings() {
        let conn = conn_with_source();
        for p in ["/d/a.txt", "/d/sub/b.txt", "/d2/c.txt", "/d"] {
            upsert_document(&conn, 1, &doc(p, 1), 1).unwrap();
        }
        let n = mark_deleted_under(&conn, 1, "/d", '/').unwrap();
        assert_eq!(n, 3);
        assert!(doc_state(&conn, "/d/sub/b.txt").0);
        assert!(!doc_state(&conn, "/d2/c.txt").0, "/d2 only shares a prefix");
    }

    #[test]
    fn delete_keeps_indexed_at_for_index_cleanup() {
        let conn = conn_with_source();
        upsert_document(&conn, 1, &doc("/a.txt", 10), 100).unwrap();
        mark_indexed(&conn, "/a.txt");
        mark_deleted_under(&conn, 1, "/a.txt", '/').unwrap();
        assert_eq!(doc_state(&conn, "/a.txt"), (true, Some(50), Some(100)));
        assert_eq!(count_documents(&conn).unwrap(), 0);
    }

    #[test]
    fn sweep_deletes_only_documents_the_crawl_missed() {
        let conn = conn_with_source();
        upsert_document(&conn, 1, &doc("/old.txt", 1), 100).unwrap();
        upsert_document(&conn, 1, &doc("/kept.txt", 1), 100).unwrap();
        // A new crawl starts at 200 and only finds kept.txt.
        upsert_document(&conn, 1, &doc("/kept.txt", 1), 200).unwrap();
        assert_eq!(sweep_unseen(&conn, 1, 200).unwrap(), 1);
        assert!(doc_state(&conn, "/old.txt").0);
        assert!(!doc_state(&conn, "/kept.txt").0);
    }

    // ---------- sources ----------

    #[test]
    fn source_status_round_trips() {
        let conn = conn_with_source();
        set_source_status(&conn, 1, SourceStatus::Syncing).unwrap();
        assert_eq!(get_source(&conn, 1).unwrap().status, SourceStatus::Syncing);
    }

    #[test]
    fn missing_source_is_not_found() {
        let conn = open_in_memory().unwrap();
        assert!(matches!(get_source(&conn, 9), Err(AppError::NotFound(_))));
        assert!(matches!(
            delete_source(&conn, 9),
            Err(AppError::NotFound(_))
        ));
    }

    #[test]
    fn exclusion_patterns_include_global_ones() {
        let conn = conn_with_source();
        conn.execute("INSERT INTO sources (id, kind) VALUES (2, 'local')", [])
            .unwrap();
        conn.execute_batch(
            "INSERT INTO exclusions (source_id, pattern) VALUES (NULL, '*.tmp');
             INSERT INTO exclusions (source_id, pattern) VALUES (1, 'build');
             INSERT INTO exclusions (source_id, pattern) VALUES (2, 'other');",
        )
        .unwrap();
        assert_eq!(
            list_exclusion_patterns(&conn, 1).unwrap(),
            vec!["*.tmp".to_string(), "build".to_string()]
        );
    }

    #[test]
    fn sync_cursor_round_trips_and_updates() {
        let conn = conn_with_source();
        assert_eq!(get_sync_cursor(&conn, 1).unwrap(), None);

        set_sync_cursor(&conn, 1, "delta-token-1", 100).unwrap();
        assert_eq!(
            get_sync_cursor(&conn, 1).unwrap(),
            Some("delta-token-1".to_string())
        );
        let state = get_sync_state(&conn, 1).unwrap().unwrap();
        assert_eq!(state.0, Some("delta-token-1".to_string()));
        assert_eq!(state.1, Some(100));

        // Update cursor
        set_sync_cursor(&conn, 1, "delta-token-2", 200).unwrap();
        assert_eq!(
            get_sync_cursor(&conn, 1).unwrap(),
            Some("delta-token-2".to_string())
        );
        let updated = get_sync_state(&conn, 1).unwrap().unwrap();
        assert_eq!(updated.0, Some("delta-token-2".to_string()));
        assert_eq!(updated.1, Some(200));
    }
}
