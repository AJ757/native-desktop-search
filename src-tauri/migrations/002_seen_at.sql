-- Every crawl stamps the files it finds with seen_at. When a full crawl
-- finishes, live documents it did not stamp were deleted while the app was
-- closed, so they get marked deleted.
ALTER TABLE documents ADD COLUMN seen_at INTEGER;

-- Status counts and per-source sweeps filter on these two columns.
CREATE INDEX idx_documents_source_deleted ON documents (source_id, deleted);

UPDATE settings SET value = '2' WHERE key = 'schema_version';
