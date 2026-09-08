-- Downgrade 0006 → 0005: drop persistent circuit breaker state.
-- Breaker state is derived from observed outcomes, so discarding it only
-- costs the current cooldown windows.
DROP INDEX IF EXISTS provider_circuit_open;
DROP TABLE IF EXISTS provider_circuit;

PRAGMA user_version = 5;
