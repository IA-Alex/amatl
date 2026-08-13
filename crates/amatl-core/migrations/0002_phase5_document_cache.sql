CREATE TABLE IF NOT EXISTS document_cache (
  canonical_url TEXT NOT NULL,
  content_hash TEXT NOT NULL,
  extractor_version TEXT NOT NULL,
  payload TEXT NOT NULL,
  size_bytes INTEGER NOT NULL,
  created_at INTEGER NOT NULL,
  last_accessed INTEGER NOT NULL,
  PRIMARY KEY(canonical_url, content_hash, extractor_version)
);

CREATE INDEX IF NOT EXISTS document_cache_lru
  ON document_cache(last_accessed);

PRAGMA user_version = 2;
