CREATE TABLE sources (
    id          INTEGER PRIMARY KEY,
    kind        TEXT    NOT NULL,                 -- local, obsidian, google_drive, ...
    config      TEXT    NOT NULL DEFAULT '{}',    -- JSON text
    status      TEXT    NOT NULL DEFAULT 'idle',
    created_at  INTEGER NOT NULL DEFAULT (unixepoch())
);

CREATE TABLE documents (
    id           INTEGER PRIMARY KEY,
    source_id    INTEGER NOT NULL REFERENCES sources(id) ON DELETE CASCADE,
    external_id  TEXT    NOT NULL,                -- path for local, provider id for cloud
    title        TEXT    NOT NULL DEFAULT '',
    mime         TEXT,
    modified_at  INTEGER,
    content_hash TEXT,
    indexed_at   INTEGER,
    deleted      INTEGER NOT NULL DEFAULT 0,
    UNIQUE (source_id, external_id)               -- ingest upserts rely on this
);

CREATE TABLE chunks (
    id           INTEGER PRIMARY KEY,
    document_id  INTEGER NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
    ordinal      INTEGER NOT NULL,
    start_offset INTEGER NOT NULL,
    end_offset   INTEGER NOT NULL,
    UNIQUE (document_id, ordinal)
);

CREATE TABLE sync_state (
    source_id      INTEGER PRIMARY KEY REFERENCES sources(id) ON DELETE CASCADE,
    cursor         TEXT,
    last_synced_at INTEGER
);

CREATE TABLE exclusions (
    id        INTEGER PRIMARY KEY,
    source_id INTEGER REFERENCES sources(id) ON DELETE CASCADE,  -- NULL = applies to all sources
    pattern   TEXT NOT NULL,
    UNIQUE (source_id, pattern)
);

CREATE TABLE settings (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

INSERT INTO settings (key, value) VALUES ('schema_version', '1');