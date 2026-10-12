-- Index the cache retention columns.
--
-- Both caches filter and delete by `created_at` on every write
-- (`DELETE FROM <cache> WHERE created_at < ?`) and count rows for their entry
-- cap, while only `last_accessed` was indexed. Without these indexes each
-- cache write is a full table scan that grows linearly with cache size.
CREATE INDEX IF NOT EXISTS provider_search_cache_created
  ON provider_search_cache(created_at);

CREATE INDEX IF NOT EXISTS document_cache_created
  ON document_cache(created_at);

PRAGMA user_version = 8;
