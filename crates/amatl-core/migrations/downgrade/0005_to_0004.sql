-- Downgrade 0005 → 0004: remove request_id column from telemetry.
-- SQLite does not support DROP COLUMN directly in older versions;
-- we recreate the table without the column.
CREATE TABLE telemetry_observations_new (
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
INSERT INTO telemetry_observations_new
    (id, observed_at, provider, category, outcome, latency_ms,
     total_results, unique_results, duplicate_ratio, top_k_contribution,
     diversity, cost_units)
SELECT id, observed_at, provider, category, outcome, latency_ms,
       total_results, unique_results, duplicate_ratio, top_k_contribution,
       diversity, cost_units
FROM telemetry_observations;
DROP TABLE telemetry_observations;
ALTER TABLE telemetry_observations_new RENAME TO telemetry_observations;
DELETE FROM amatl_schema_migrations WHERE version = 5;
PRAGMA user_version = 4;
