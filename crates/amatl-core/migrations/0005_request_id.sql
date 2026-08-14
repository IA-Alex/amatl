-- Migration 0005: request_id column for trace correlation in telemetry.
ALTER TABLE telemetry_observations ADD COLUMN request_id TEXT;

PRAGMA user_version = 5;