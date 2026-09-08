-- Domain persistence: search history and saved documents.
-- Enables cross-session reuse of past searches and deep-fetched documents.

CREATE TABLE IF NOT EXISTS search_history (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  normalized_query TEXT NOT NULL,
  raw_query TEXT NOT NULL,
  category TEXT,
  provider_count INTEGER NOT NULL DEFAULT 0,
  total_results INTEGER NOT NULL DEFAULT 0,
  deep_fetches INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL,
  surface TEXT NOT NULL DEFAULT 'cli'
);

CREATE INDEX IF NOT EXISTS search_history_query
  ON search_history(normalized_query, created_at);

CREATE INDEX IF NOT EXISTS search_history_time
  ON search_history(created_at);

CREATE TABLE IF NOT EXISTS saved_documents (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  canonical_url TEXT NOT NULL,
  title TEXT,
  snippet TEXT,
  content_hash TEXT NOT NULL,
  extractor_version TEXT NOT NULL,
  payload TEXT NOT NULL,
  size_bytes INTEGER NOT NULL,
  saved_at INTEGER NOT NULL,
  source_query TEXT,
  tags TEXT NOT NULL DEFAULT '[]'
);

CREATE UNIQUE INDEX IF NOT EXISTS saved_documents_url_hash_version
  ON saved_documents(canonical_url, content_hash, extractor_version);

CREATE INDEX IF NOT EXISTS saved_documents_time
  ON saved_documents(saved_at);

PRAGMA user_version = 3;
