-- Migration 0006: persistent provider circuit breaker state.
-- Keeps a source that is failing from being retried on every request, and
-- keeps that decision across restarts instead of relearning it from zero.

CREATE TABLE IF NOT EXISTS provider_circuit (
  provider TEXT PRIMARY KEY,
  consecutive_failures INTEGER NOT NULL DEFAULT 0,
  opened_at INTEGER,
  open_until INTEGER,
  updated_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS provider_circuit_open
  ON provider_circuit(open_until);

PRAGMA user_version = 6;
