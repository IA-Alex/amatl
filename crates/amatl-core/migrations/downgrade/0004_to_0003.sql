-- Downgrade from version 4 to version 3.
-- Note: SQLite does not support DROP COLUMN in older versions,
-- so we recreate the table without the revalidation columns.
CREATE TABLE IF NOT EXISTS document_cache_downgrade (
  canonical_url TEXT NOT NULL,
  content_hash TEXT NOT NULL,
  extractor_version TEXT NOT NULL,
  payload TEXT NOT NULL,
  size_bytes INTEGER NOT NULL,
  created_at INTEGER NOT NULL,
  last_accessed INTEGER NOT NULL,
  PRIMARY KEY(canonical_url, content_hash, extractor_version)
);

INSERT INTO document_cache_downgrade
  (canonical_url, content_hash, extractor_version, payload, size_bytes, created_at, last_accessed)
SELECT canonical_url, content_hash, extractor_version, payload, size_bytes, created_at, last_accessed
FROM document_cache;

DROP TABLE document_cache;
ALTER TABLE document_cache_downgrade RENAME TO document_cache;

CREATE INDEX IF NOT EXISTS document_cache_lru
  ON document_cache(last_accessed);

PRAGMA user_version = 3;
