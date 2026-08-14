//! Persistent provider circuit breaker.
//!
//! Telemetry answers "how good has this source been?"; the breaker answers the
//! narrower operational question "should we call it right now?". The two are
//! deliberately separate: telemetry is a decaying quality signal used for
//! routing, while the breaker is a hard, short-lived stop that protects the
//! search budget from a source that is currently failing.
//!
//! State survives restarts. Consecutive failures and the cooldown deadline are
//! written to SQLite when persistence is enabled, so a process that restarts
//! inside a cooldown window does not immediately spend its budget rediscovering
//! that the source is down. Without persistence the breaker still works, in
//! memory only.
//!
//! The breaker never invents availability: it only ever *removes* a source from
//! a round. A closed breaker means "not currently blocked", not "healthy".

use crate::storage::{SqliteStorage, StoredCircuitRecord};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

/// Trip and recovery limits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct CircuitPolicy {
    pub enabled: bool,
    /// Consecutive failures that open the circuit.
    pub failure_threshold: u32,
    /// How long the circuit stays open before one probe is allowed.
    pub open_seconds: u64,
}

impl Default for CircuitPolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            failure_threshold: 3,
            open_seconds: 60,
        }
    }
}

impl CircuitPolicy {
    pub fn validate(&self) -> Result<(), &'static str> {
        if !(1..=100).contains(&self.failure_threshold) {
            return Err("circuit_breaker.failure_threshold must be between 1 and 100");
        }
        if !(1..=3_600).contains(&self.open_seconds) {
            return Err("circuit_breaker.open_seconds must be between 1 and 3600");
        }
        Ok(())
    }
}

/// What the breaker says about one provider at a point in time.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CircuitState {
    /// Calls proceed normally.
    Closed,
    /// Calls are refused until the cooldown expires.
    Open,
    /// Cooldown expired; the next call is a probe that decides the outcome.
    HalfOpen,
}

impl CircuitState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Closed => "closed",
            Self::Open => "open",
            Self::HalfOpen => "half_open",
        }
    }

    /// Whether a call may be attempted in this state.
    pub const fn allows_call(self) -> bool {
        matches!(self, Self::Closed | Self::HalfOpen)
    }
}

/// Public view of one provider's breaker.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CircuitSnapshot {
    pub provider: String,
    pub state: CircuitState,
    pub consecutive_failures: u32,
    /// Unix seconds until which calls stay refused, when open.
    pub open_until: Option<i64>,
}

#[derive(Clone, Debug, Default)]
struct CircuitRecord {
    consecutive_failures: u32,
    opened_at: Option<i64>,
    open_until: Option<i64>,
}

/// Circuit breaker shared by every surface of one service instance.
#[derive(Clone)]
pub struct ProviderCircuit {
    policy: CircuitPolicy,
    storage: Option<SqliteStorage>,
    records: Arc<Mutex<BTreeMap<String, CircuitRecord>>>,
}

impl ProviderCircuit {
    /// In-memory breaker with no persistence.
    pub fn new(policy: CircuitPolicy) -> Self {
        Self {
            policy,
            storage: None,
            records: Arc::new(Mutex::new(BTreeMap::new())),
        }
    }

    /// Breaker restored from SQLite when persistence is available.
    ///
    /// A restore failure is not fatal: the breaker starts closed rather than
    /// blocking sources on a storage problem.
    pub async fn restored(policy: CircuitPolicy, storage: Option<SqliteStorage>) -> Self {
        let breaker = Self {
            policy,
            storage,
            records: Arc::new(Mutex::new(BTreeMap::new())),
        };
        if let Some(storage) = &breaker.storage {
            if let Ok(rows) = storage.circuit_load().await {
                let restored = rows
                    .into_iter()
                    .map(|row| {
                        (
                            row.provider,
                            CircuitRecord {
                                consecutive_failures: row.consecutive_failures,
                                opened_at: row.opened_at,
                                open_until: row.open_until,
                            },
                        )
                    })
                    .collect();
                if let Ok(mut records) = breaker.records.lock() {
                    *records = restored;
                }
            }
        }
        breaker
    }

    pub fn policy(&self) -> CircuitPolicy {
        self.policy
    }

    /// State of one provider at `now` (unix seconds).
    pub fn state(&self, provider: &str, now: i64) -> CircuitState {
        if !self.policy.enabled {
            return CircuitState::Closed;
        }
        let Ok(records) = self.records.lock() else {
            return CircuitState::Closed;
        };
        match records.get(provider).and_then(|record| record.open_until) {
            Some(until) if now < until => CircuitState::Open,
            Some(_) => CircuitState::HalfOpen,
            None => CircuitState::Closed,
        }
    }

    /// Whether a call to `provider` may be attempted now.
    pub fn allows_call(&self, provider: &str, now: i64) -> bool {
        self.state(provider, now).allows_call()
    }

    /// Record one call outcome, opening or closing the circuit as needed.
    pub async fn record(&self, provider: &str, success: bool, now: i64) {
        if !self.policy.enabled {
            return;
        }
        let updated = {
            let Ok(mut records) = self.records.lock() else {
                return;
            };
            let record = records.entry(provider.to_owned()).or_default();
            if success {
                *record = CircuitRecord::default();
            } else {
                record.consecutive_failures = record.consecutive_failures.saturating_add(1);
                if record.consecutive_failures >= self.policy.failure_threshold {
                    record.opened_at = Some(now);
                    record.open_until = Some(now.saturating_add(self.policy.open_seconds as i64));
                }
            }
            record.clone()
        };
        if !success && updated.open_until.is_some_and(|until| until > now) {
            tracing::warn!(
                target: "amatl::providers",
                provider,
                consecutive_failures = updated.consecutive_failures,
                open_until = updated.open_until,
                "provider circuit opened; the source is skipped until the cooldown expires"
            );
        }
        if let Some(storage) = &self.storage {
            let _ = storage
                .circuit_put(&StoredCircuitRecord {
                    provider: provider.to_owned(),
                    consecutive_failures: updated.consecutive_failures,
                    opened_at: updated.opened_at,
                    open_until: updated.open_until,
                    updated_at: now,
                })
                .await;
        }
    }

    /// Current state of every provider the breaker has observed.
    pub fn snapshots(&self, now: i64) -> Vec<CircuitSnapshot> {
        let Ok(records) = self.records.lock() else {
            return vec![];
        };
        records
            .iter()
            .map(|(provider, record)| CircuitSnapshot {
                provider: provider.clone(),
                state: match record.open_until {
                    _ if !self.policy.enabled => CircuitState::Closed,
                    Some(until) if now < until => CircuitState::Open,
                    Some(_) => CircuitState::HalfOpen,
                    None => CircuitState::Closed,
                },
                consecutive_failures: record.consecutive_failures,
                open_until: record.open_until,
            })
            .collect()
    }

    /// Close every circuit, for an operator that fixed the cause.
    pub async fn reset(&self) {
        if let Ok(mut records) = self.records.lock() {
            records.clear();
        }
        if let Some(storage) = &self.storage {
            let _ = storage.circuit_clear().await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy() -> CircuitPolicy {
        CircuitPolicy {
            enabled: true,
            failure_threshold: 2,
            open_seconds: 30,
        }
    }

    #[tokio::test]
    async fn opens_after_consecutive_failures_and_recovers_after_the_cooldown() {
        let breaker = ProviderCircuit::new(policy());
        breaker.record("brave", false, 1_000).await;
        assert_eq!(breaker.state("brave", 1_000), CircuitState::Closed);
        breaker.record("brave", false, 1_001).await;
        assert_eq!(breaker.state("brave", 1_001), CircuitState::Open);
        assert!(!breaker.allows_call("brave", 1_010));
        // The cooldown expires into a single probe, not straight back to closed.
        assert_eq!(breaker.state("brave", 1_031), CircuitState::HalfOpen);
        assert!(breaker.allows_call("brave", 1_031));
        breaker.record("brave", true, 1_032).await;
        assert_eq!(breaker.state("brave", 1_032), CircuitState::Closed);
    }

    #[tokio::test]
    async fn a_failed_probe_reopens_the_circuit() {
        let breaker = ProviderCircuit::new(policy());
        breaker.record("mojeek", false, 10).await;
        breaker.record("mojeek", false, 11).await;
        assert_eq!(breaker.state("mojeek", 41), CircuitState::HalfOpen);
        breaker.record("mojeek", false, 41).await;
        assert_eq!(breaker.state("mojeek", 42), CircuitState::Open);
    }

    #[tokio::test]
    async fn a_disabled_breaker_never_blocks_a_provider() {
        let breaker = ProviderCircuit::new(CircuitPolicy {
            enabled: false,
            ..policy()
        });
        for tick in 0..10 {
            breaker.record("brave", false, tick).await;
        }
        assert_eq!(breaker.state("brave", 10), CircuitState::Closed);
        assert!(breaker.allows_call("brave", 10));
    }

    #[test]
    fn policy_limits_are_validated() {
        assert!(CircuitPolicy::default().validate().is_ok());
        assert!(CircuitPolicy {
            failure_threshold: 0,
            ..CircuitPolicy::default()
        }
        .validate()
        .is_err());
        assert!(CircuitPolicy {
            open_seconds: 0,
            ..CircuitPolicy::default()
        }
        .validate()
        .is_err());
    }

    #[tokio::test]
    async fn state_survives_a_restart_when_persistence_is_available() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "amatl-circuit-{}-{nonce}.sqlite3",
            std::process::id()
        ));
        let storage = SqliteStorage::open(&path).await.unwrap();
        let breaker = ProviderCircuit::restored(policy(), Some(storage.clone())).await;
        breaker.record("brave", false, 5_000).await;
        breaker.record("brave", false, 5_001).await;
        assert_eq!(breaker.state("brave", 5_002), CircuitState::Open);

        let restarted = ProviderCircuit::restored(policy(), Some(storage.clone())).await;
        assert_eq!(restarted.state("brave", 5_002), CircuitState::Open);
        assert_eq!(restarted.snapshots(5_002)[0].consecutive_failures, 2);

        restarted.reset().await;
        let cleared = ProviderCircuit::restored(policy(), Some(storage)).await;
        assert_eq!(cleared.state("brave", 5_002), CircuitState::Closed);
        let _ = std::fs::remove_file(path);
    }
}
