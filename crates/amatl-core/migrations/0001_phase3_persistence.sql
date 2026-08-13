CREATE TABLE IF NOT EXISTS provider_search_cache (
  provider TEXT NOT NULL,
  adapter_version TEXT NOT NULL,
  normalized_query TEXT NOT NULL,
  structured_filters TEXT NOT NULL,
  payload TEXT NOT NULL,
  size_bytes INTEGER NOT NULL,
  created_at INTEGER NOT NULL,
  last_accessed INTEGER NOT NULL,
  PRIMARY KEY(provider, adapter_version, normalized_query, structured_filters)
);

CREATE INDEX IF NOT EXISTS provider_search_cache_lru
  ON provider_search_cache(last_accessed);

CREATE TABLE IF NOT EXISTS telemetry_observations (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  observed_at INTEGER NOT NULL,
  provider TEXT NOT NULL,
  category TEXT NOT NULL,
  outcome TEXT NOT NULL,
  latency_ms INTEGER NOT NULL,
  total_results INTEGER NOT NULL,
  unique_results INTEGER NOT NULL,
  duplicate_ratio REAL NOT NULL,
  top_k_contribution REAL NOT NULL,
  diversity REAL NOT NULL,
  cost_units INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS telemetry_window
  ON telemetry_observations(observed_at, provider, category);

PRAGMA user_version = 1;
