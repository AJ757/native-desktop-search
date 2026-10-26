CREATE TABLE document_embeddings (
    document_id INTEGER PRIMARY KEY
        REFERENCES documents(id) ON DELETE CASCADE,
    content TEXT NOT NULL,
    embedding BLOB NOT NULL,
    dimensions INTEGER NOT NULL,
    updated_at INTEGER NOT NULL DEFAULT (unixepoch())
);

CREATE INDEX idx_document_embeddings_dimensions
    ON document_embeddings (dimensions);

UPDATE settings SET value = '3' WHERE key = 'schema_version';
