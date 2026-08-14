-- Migration 0007: queryable security audit trail.
-- Rejections at the HTTP edge were observable only in process logs; this table
-- makes them durable and queryable from AMATL itself, with its own retention.

CREATE TABLE IF NOT EXISTS security_events (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  observed_at INTEGER NOT NULL,
  event TEXT NOT NULL,
  request_id TEXT,
  client_id TEXT,
  path TEXT,
  client_ip TEXT
);

CREATE INDEX IF NOT EXISTS security_events_time
  ON security_events(observed_at);

CREATE INDEX IF NOT EXISTS security_events_kind
  ON security_events(event, observed_at);

PRAGMA user_version = 7;
