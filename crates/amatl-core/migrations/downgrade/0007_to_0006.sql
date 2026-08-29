-- Downgrade 0007 → 0006: drop the persisted audit trail.
-- Events remain in process logs; only the queryable copy is discarded.
DROP INDEX IF EXISTS security_events_kind;
DROP INDEX IF EXISTS security_events_time;
DROP TABLE IF EXISTS security_events;

PRAGMA user_version = 6;
