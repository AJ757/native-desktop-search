//! SQLite access: connection setup, migrations and queries.
//!
//! SQLite is the source of truth. The Tantivy index and the vector store
//! must always be rebuildable from what is stored here.

use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

use rusqlite::{params, Connection, Row};
use rusqlite_migration::{Migrations, M};

use serde::de::DeserializeOwned;

use crate::types::{AddSourceRequest, Source};

use crate::error::Result;

fn migrations() -> Migrations<'static> {
    Migrations::new(vec![
        M::up(include_str!("../migrations/001_init.sql")),
        // 002 goes here later; never edit an applied migration.
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
        let v1 = user_version(&conn);
        migrations().to_latest(&mut conn).unwrap();
        assert_eq!(user_version(&conn), v1);
        assert_eq!(v1, 1);
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
}
