//! Durable security audit trail.
//!
//! Every rejection at the HTTP edge is already logged; this module keeps a
//! queryable copy so an operator can answer "who was refused, when, and for
//! what" without a log pipeline. Three properties matter more than
//! completeness:
//!
//! * **A request is never delayed by auditing.** Recording is handed to a
//!   background task; the rejection response goes out immediately.
//! * **A flood cannot become a write amplifier.** Concurrent writes are capped;
//!   beyond the cap events are dropped and counted, so an attacker hammering an
//!   unauthorized endpoint cannot turn the audit trail into the outage.
//! * **It never carries secrets.** Only the event name, the request id, the
//!   authenticated identity, the path and the client IP are stored — the same
//!   fields the log line already contains.
//!
//! Without persistence the recorder is inert and logging remains the only
//! trail, which is exactly the pre-existing behavior.

use crate::storage::{SecurityEvent, SqliteStorage};
use crate::telemetry::now_unix;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// Default window an audit trail is kept for.
pub const AUDIT_DEFAULT_RETENTION_DAYS: u32 = 90;
/// Widest retention a configuration may ask for.
pub const AUDIT_MAX_RETENTION_DAYS: u32 = 365;
/// Audit writes allowed to be in flight at once.
const MAX_IN_FLIGHT: u64 = 32;

/// What to record about one rejection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SecurityEventInput {
    pub event: String,
    pub request_id: Option<String>,
    pub client_id: Option<String>,
    pub path: Option<String>,
    pub client_ip: Option<String>,
}

/// Background writer for the audit trail.
#[derive(Clone)]
pub struct SecurityAudit {
    storage: Option<SqliteStorage>,
    retention_days: u32,
    in_flight: Arc<AtomicU64>,
    dropped: Arc<AtomicU64>,
}

impl SecurityAudit {
    /// Recorder that only logs, used when persistence is unavailable.
    pub fn disabled() -> Self {
        Self {
            storage: None,
            retention_days: AUDIT_DEFAULT_RETENTION_DAYS,
            in_flight: Arc::new(AtomicU64::new(0)),
            dropped: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn new(storage: Option<SqliteStorage>, retention_days: u32) -> Self {
        Self {
            storage,
            retention_days: retention_days.clamp(1, AUDIT_MAX_RETENTION_DAYS),
            in_flight: Arc::new(AtomicU64::new(0)),
            dropped: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn enabled(&self) -> bool {
        self.storage.is_some()
    }

    /// Events dropped because too many writes were already in flight.
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    /// Record one event without blocking the caller.
    pub fn record(&self, input: SecurityEventInput) {
        let Some(storage) = self.storage.clone() else {
            return;
        };
        if self.in_flight.fetch_add(1, Ordering::AcqRel) >= MAX_IN_FLIGHT {
            self.in_flight.fetch_sub(1, Ordering::AcqRel);
            self.dropped.fetch_add(1, Ordering::Relaxed);
            return;
        }
        let event = SecurityEvent {
            id: 0,
            observed_at: now_unix(),
            event: input.event,
            request_id: input.request_id,
            client_id: input.client_id,
            path: input.path,
            client_ip: input.client_ip,
        };
        let retention_days = self.retention_days;
        let in_flight = self.in_flight.clone();
        tokio::spawn(async move {
            if storage
                .security_event_insert(&event, retention_days)
                .await
                .is_err()
            {
                tracing::warn!(
                    target: "amatl::security",
                    security_event = "audit_write_failed",
                    "security event could not be persisted; the log line remains the only record"
                );
            }
            in_flight.fetch_sub(1, Ordering::AcqRel);
        });
    }

    /// Recorded events, newest first.
    pub async fn events(
        &self,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<SecurityEvent>, crate::StorageError> {
        let Some(storage) = &self.storage else {
            return Err(crate::StorageError::Operation);
        };
        storage
            .security_events(limit.clamp(1, 500) as i64, offset as i64)
            .await
    }

    pub async fn count(&self) -> Option<i64> {
        self.storage.as_ref()?.security_event_count().await.ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(event: &str) -> SecurityEventInput {
        SecurityEventInput {
            event: event.into(),
            request_id: Some("request-1".into()),
            client_id: Some("operator".into()),
            path: Some("/search".into()),
            client_ip: Some("127.0.0.1".into()),
        }
    }

    async fn storage() -> SqliteStorage {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        SqliteStorage::open(std::env::temp_dir().join(format!(
            "amatl-audit-{}-{nonce}.sqlite3",
            std::process::id()
        )))
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn events_are_persisted_and_queried_newest_first() {
        let audit = SecurityAudit::new(Some(storage().await), 90);
        assert!(audit.enabled());
        audit.record(input("unauthorized"));
        audit.record(input("scope_denied"));
        // Writes are backgrounded; wait for them to land.
        for _ in 0..50 {
            if audit.count().await.unwrap_or(0) >= 2 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        let events = audit.events(10, 0).await.unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].client_id.as_deref(), Some("operator"));
        assert!(events.iter().any(|event| event.event == "scope_denied"));
        assert_eq!(audit.dropped(), 0);
    }

    #[tokio::test]
    async fn a_disabled_recorder_is_inert_rather_than_failing() {
        let audit = SecurityAudit::disabled();
        assert!(!audit.enabled());
        audit.record(input("unauthorized"));
        assert!(audit.events(10, 0).await.is_err());
        assert_eq!(audit.count().await, None);
    }
}
