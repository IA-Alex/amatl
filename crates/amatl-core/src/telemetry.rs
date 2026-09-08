use crate::model::{Category, ProviderErrorKind};
use crate::storage::{SqliteStorage, StoredTelemetryObservation};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

const DECAY_HALF_LIFE_DAYS: f64 = 30.0;

/// Minimum allowed telemetry retention window (7 days).
pub const TELEMETRY_MIN_RETENTION_DAYS: u32 = 7;
/// Maximum allowed telemetry retention window (365 days).
pub const TELEMETRY_MAX_RETENTION_DAYS: u32 = 365;
/// Default retention window when no config is provided.
pub const TELEMETRY_DEFAULT_RETENTION_DAYS: u32 = 30;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProviderValueState {
    Bootstrap,
    Learning,
    Mature,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProviderHealth {
    Healthy,
    Degraded,
    Unavailable,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TelemetryOutcome {
    Success,
    Partial,
    Error,
    Timeout,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct TelemetryObservation {
    pub observed_at: i64,
    pub provider: String,
    pub category: Category,
    pub outcome: TelemetryOutcome,
    pub latency_ms: u64,
    pub total_results: u64,
    pub unique_results: u64,
    pub duplicate_ratio: f64,
    pub top_k_contribution: f64,
    pub diversity: f64,
    pub cost_units: u64,
    /// Correlates this observation with the originating HTTP request, CLI
    /// invocation, or MCP session so traces can be reconstructed end-to-end.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
}

pub(crate) struct ProviderTelemetryInput<'a> {
    pub provider: String,
    pub category: Category,
    pub latency_ms: u64,
    pub total_results: usize,
    pub unique_results: usize,
    pub error: Option<&'a ProviderErrorKind>,
    pub partial: bool,
    pub estimated_cost: Option<u64>,
    /// Correlates this observation with the originating request.
    pub request_id: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ProviderValueSnapshot {
    pub provider: String,
    pub category: Option<Category>,
    pub state: ProviderValueState,
    pub health: ProviderHealth,
    pub window_days: u32,
    pub sample: u64,
    pub weighted_sample: f64,
    pub success_rate: f64,
    pub timeout_rate: f64,
    pub average_latency_ms: f64,
    pub average_cost_units: f64,
    pub average_unique_results: f64,
    pub duplicate_ratio: f64,
    pub top_k_contribution: f64,
    pub diversity: f64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TelemetryStatus {
    pub in_memory_observations: usize,
    pub persistence_enabled: bool,
    pub persistence_failures: u64,
}

#[derive(Clone, Default)]
pub struct InMemoryTelemetry {
    state: Arc<Mutex<TelemetryState>>,
    storage: Option<SqliteStorage>,
    window_seconds: i64,
}

#[derive(Default)]
struct TelemetryState {
    observations: VecDeque<TelemetryObservation>,
    persistence_failures: u64,
}

impl InMemoryTelemetry {
    pub fn new() -> Self {
        Self {
            window_seconds: TELEMETRY_DEFAULT_RETENTION_DAYS as i64 * 86_400,
            ..Default::default()
        }
    }

    pub fn with_retention_days(retention_days: u32) -> Self {
        Self {
            window_seconds: (retention_days as i64).saturating_mul(86_400),
            ..Default::default()
        }
    }

    pub async fn with_optional_storage(storage: Option<SqliteStorage>) -> Self {
        let telemetry = Self {
            state: Arc::new(Mutex::new(TelemetryState::default())),
            storage,
            window_seconds: TELEMETRY_DEFAULT_RETENTION_DAYS as i64 * 86_400,
        };
        telemetry.restore_best_effort(now_unix()).await;
        telemetry
    }

    pub async fn with_storage_and_retention(
        storage: Option<SqliteStorage>,
        retention_days: u32,
    ) -> Self {
        let telemetry = Self {
            state: Arc::new(Mutex::new(TelemetryState::default())),
            storage,
            window_seconds: (retention_days as i64).saturating_mul(86_400),
        };
        telemetry.restore_best_effort(now_unix()).await;
        telemetry
    }

    /// Record one observation in memory and, when configured, in `storage`.
    ///
    /// The two writes are deliberately **not** transactional. Memory is the
    /// source of truth `snapshot`/`snapshot_for_routing` read from; `storage`
    /// is a durability backup that `restore_best_effort` replays into memory
    /// on the next startup. Making them atomic — rolling the memory write
    /// back when the storage write fails, or vice versa — would mean an
    /// ordinary SQLite hiccup (already expected and non-fatal everywhere else
    /// in this codebase; see the module-level invariant that storage failures
    /// never invalidate Search) starts also discarding the in-memory sample
    /// that routing depends on right now. That trades a purely cosmetic
    /// inconsistency (storage briefly ahead of memory, self-healed by
    /// `restore_best_effort` on the next restart) for a real one: a flaky
    /// disk degrading routing decisions. `lock_state` already closes the gap
    /// this method used to have — a poisoned mutex no longer skips the memory
    /// write silently and permanently (see its doc comment and
    /// `survives_a_poisoned_state_mutex_without_losing_future_observations`);
    /// what remains is two independent stores with intentionally independent
    /// failure domains, not a bug.
    pub async fn record(&self, mut observation: TelemetryObservation) {
        observation.duplicate_ratio = observation.duplicate_ratio.clamp(0.0, 1.0);
        observation.top_k_contribution = observation.top_k_contribution.clamp(0.0, 1.0);
        observation.diversity = observation.diversity.clamp(0.0, 1.0);
        let now = observation.observed_at;
        let cutoff = now - self.window_seconds;
        {
            let mut state = self.lock_state();
            state.observations.push_back(observation.clone());
            prune_memory(&mut state.observations, cutoff);
        }
        if let Some(storage) = &self.storage {
            if storage
                .telemetry_insert(&observation.clone().into())
                .await
                .is_err()
            {
                self.mark_persistence_failure();
            }
            if storage.telemetry_prune(cutoff).await.is_err() {
                self.mark_persistence_failure();
            }
        }
    }

    pub fn snapshot_global(&self, provider: &str, now: i64) -> ProviderValueSnapshot {
        self.snapshot(provider, None, now)
    }

    pub fn snapshot_by_category(
        &self,
        provider: &str,
        category: Category,
        now: i64,
    ) -> ProviderValueSnapshot {
        self.snapshot(provider, Some(category), now)
    }

    pub fn snapshot_for_routing(
        &self,
        provider: &str,
        category: Category,
        now: i64,
    ) -> ProviderValueSnapshot {
        let category_snapshot = self.snapshot_by_category(provider, category.clone(), now);
        if category_snapshot.sample >= 100 {
            return category_snapshot;
        }
        let global = self.snapshot_global(provider, now);
        if global.sample == 0 {
            return category_snapshot;
        }
        if category_snapshot.sample == 0 {
            return ProviderValueSnapshot {
                category: Some(category),
                ..global
            };
        }
        let global_weight = global.weighted_sample;
        let category_weight = category_snapshot.weighted_sample;
        let total_weight = global_weight + category_weight;
        let blend = |global_value: f64, category_value: f64| {
            if total_weight == 0.0 {
                0.0
            } else {
                (global_value * global_weight + category_value * category_weight) / total_weight
            }
        };
        let success_rate = blend(global.success_rate, category_snapshot.success_rate);
        let timeout_rate = blend(global.timeout_rate, category_snapshot.timeout_rate);
        ProviderValueSnapshot {
            provider: provider.into(),
            category: Some(category),
            state: global.state,
            health: health_for(global.sample, success_rate, timeout_rate),
            window_days: self.window_days(),
            sample: global.sample,
            weighted_sample: total_weight,
            success_rate,
            timeout_rate,
            average_latency_ms: blend(
                global.average_latency_ms,
                category_snapshot.average_latency_ms,
            ),
            average_cost_units: blend(
                global.average_cost_units,
                category_snapshot.average_cost_units,
            ),
            average_unique_results: blend(
                global.average_unique_results,
                category_snapshot.average_unique_results,
            ),
            duplicate_ratio: blend(global.duplicate_ratio, category_snapshot.duplicate_ratio),
            top_k_contribution: blend(
                global.top_k_contribution,
                category_snapshot.top_k_contribution,
            ),
            diversity: blend(global.diversity, category_snapshot.diversity),
        }
    }

    pub fn snapshots(&self, now: i64) -> Vec<ProviderValueSnapshot> {
        let providers = self
            .lock_state()
            .observations
            .iter()
            .map(|observation| observation.provider.clone())
            .collect::<std::collections::BTreeSet<_>>();
        providers
            .into_iter()
            .map(|provider| self.snapshot_global(&provider, now))
            .collect()
    }

    pub fn status(&self) -> TelemetryStatus {
        let state = self.lock_state();
        TelemetryStatus {
            in_memory_observations: state.observations.len(),
            persistence_enabled: self.storage.is_some(),
            persistence_failures: state.persistence_failures,
        }
    }

    fn snapshot(
        &self,
        provider: &str,
        category: Option<Category>,
        now: i64,
    ) -> ProviderValueSnapshot {
        let cutoff = now - self.window_seconds;
        let observations = self
            .lock_state()
            .observations
            .iter()
            .filter(|item| {
                item.provider == provider
                    && item.observed_at >= cutoff
                    && category
                        .as_ref()
                        .is_none_or(|value| &item.category == value)
            })
            .cloned()
            .collect::<Vec<_>>();
        calculate_snapshot(provider, category, &observations, now, self.window_days())
    }

    async fn restore_best_effort(&self, now: i64) {
        let Some(storage) = &self.storage else { return };
        match storage.telemetry_load(now - self.window_seconds).await {
            Ok(observations) => {
                self.lock_state().observations = observations
                    .into_iter()
                    .filter_map(TelemetryObservation::try_from_stored)
                    .collect();
            }
            Err(_) => self.mark_persistence_failure(),
        }
    }

    fn mark_persistence_failure(&self) {
        let mut state = self.lock_state();
        state.persistence_failures = state.persistence_failures.saturating_add(1);
    }

    /// Lock the shared in-memory state, recovering it if a previous holder
    /// panicked mid-access.
    ///
    /// `record` writes to memory and to `storage` (when configured)
    /// independently and deliberately: a storage failure must not make an
    /// observation invisible to routing, and a poisoned in-memory lock must
    /// not make every later observation invisible either — the two stores
    /// already have distinct failure semantics (storage failures are counted
    /// in `persistence_failures` and never treated as fatal to `record`,
    /// `snapshot`, or `snapshots`). The `VecDeque`/counter mutations guarded
    /// by this mutex only push, retain, or increment; none of them can leave
    /// the state in a form that would be unsafe to keep using after a panic,
    /// so recovering — rather than letting every subsequent call silently see
    /// an empty state — restores the same in-memory bookkeeping that
    /// `storage` keeps receiving, instead of leaving telemetry permanently
    /// blind to memory-only observations for the rest of the process.
    fn lock_state(&self) -> std::sync::MutexGuard<'_, TelemetryState> {
        self.state.lock().unwrap_or_else(|error| {
            tracing::error!(
                target: "amatl::telemetry",
                security_event = "mutex_poisoned",
                component = "in_memory_state",
                "recovered a poisoned telemetry state mutex; a prior operation must have panicked while holding it"
            );
            error.into_inner()
        })
    }

    fn window_days(&self) -> u32 {
        (self.window_seconds / 86_400) as u32
    }
}

impl TelemetryObservation {
    pub(crate) fn from_provider_result(input: ProviderTelemetryInput<'_>) -> Self {
        let outcome = match input.error {
            Some(ProviderErrorKind::Timeout) => TelemetryOutcome::Timeout,
            Some(_) => TelemetryOutcome::Error,
            None if input.partial => TelemetryOutcome::Partial,
            None => TelemetryOutcome::Success,
        };
        let duplicate_ratio = if input.total_results == 0 {
            0.0
        } else {
            1.0 - input.unique_results.min(input.total_results) as f64 / input.total_results as f64
        };
        Self {
            observed_at: now_unix(),
            provider: input.provider,
            category: input.category,
            outcome,
            latency_ms: input.latency_ms,
            total_results: input.total_results as u64,
            unique_results: input.unique_results as u64,
            duplicate_ratio,
            top_k_contribution: 0.0,
            diversity: 0.0,
            cost_units: input.estimated_cost.unwrap_or(0),
            request_id: input.request_id,
        }
    }

    fn try_from_stored(value: StoredTelemetryObservation) -> Option<Self> {
        Some(Self {
            observed_at: value.observed_at,
            provider: value.provider,
            category: parse_category(&value.category)?,
            outcome: match value.outcome.as_str() {
                "success" => TelemetryOutcome::Success,
                "partial" => TelemetryOutcome::Partial,
                "error" => TelemetryOutcome::Error,
                "timeout" => TelemetryOutcome::Timeout,
                _ => return None,
            },
            latency_ms: value.latency_ms,
            total_results: value.total_results,
            unique_results: value.unique_results,
            duplicate_ratio: value.duplicate_ratio,
            top_k_contribution: value.top_k_contribution,
            diversity: value.diversity,
            cost_units: value.cost_units,
            request_id: value.request_id,
        })
    }
}

impl From<TelemetryObservation> for StoredTelemetryObservation {
    fn from(value: TelemetryObservation) -> Self {
        Self {
            observed_at: value.observed_at,
            provider: value.provider,
            category: category_name(&value.category).into(),
            outcome: match value.outcome {
                TelemetryOutcome::Success => "success",
                TelemetryOutcome::Partial => "partial",
                TelemetryOutcome::Error => "error",
                TelemetryOutcome::Timeout => "timeout",
            }
            .into(),
            latency_ms: value.latency_ms,
            total_results: value.total_results,
            unique_results: value.unique_results,
            duplicate_ratio: value.duplicate_ratio,
            top_k_contribution: value.top_k_contribution,
            diversity: value.diversity,
            cost_units: value.cost_units,
            request_id: value.request_id,
        }
    }
}

fn calculate_snapshot(
    provider: &str,
    category: Option<Category>,
    observations: &[TelemetryObservation],
    now: i64,
    window_days: u32,
) -> ProviderValueSnapshot {
    let mut weights = BTreeMap::new();
    let mut total_weight = 0.0;
    for (index, observation) in observations.iter().enumerate() {
        let age_days = (now - observation.observed_at).max(0) as f64 / 86_400.0;
        let weight = 2_f64.powf(-age_days / DECAY_HALF_LIFE_DAYS);
        weights.insert(index, weight);
        total_weight += weight;
    }
    let weighted = |extract: fn(&TelemetryObservation) -> f64| {
        if total_weight == 0.0 {
            0.0
        } else {
            observations
                .iter()
                .enumerate()
                .map(|(index, item)| weights[&index] * extract(item))
                .sum::<f64>()
                / total_weight
        }
    };
    let sample = observations.len() as u64;
    let success_rate = weighted(|item| {
        if matches!(
            item.outcome,
            TelemetryOutcome::Success | TelemetryOutcome::Partial
        ) {
            1.0
        } else {
            0.0
        }
    });
    let timeout_rate = weighted(|item| {
        if item.outcome == TelemetryOutcome::Timeout {
            1.0
        } else {
            0.0
        }
    });
    let health = health_for(sample, success_rate, timeout_rate);
    ProviderValueSnapshot {
        provider: provider.into(),
        category,
        state: match sample {
            0..=99 => ProviderValueState::Bootstrap,
            100..=499 => ProviderValueState::Learning,
            _ => ProviderValueState::Mature,
        },
        health,
        window_days,
        sample,
        weighted_sample: total_weight,
        success_rate,
        timeout_rate,
        average_latency_ms: weighted(|item| item.latency_ms as f64),
        average_cost_units: weighted(|item| item.cost_units as f64),
        average_unique_results: weighted(|item| item.unique_results as f64),
        duplicate_ratio: weighted(|item| item.duplicate_ratio),
        top_k_contribution: weighted(|item| item.top_k_contribution),
        diversity: weighted(|item| item.diversity),
    }
}

fn health_for(sample: u64, success_rate: f64, timeout_rate: f64) -> ProviderHealth {
    if sample == 0 || success_rate < 0.2 || timeout_rate >= 0.8 {
        ProviderHealth::Unavailable
    } else if success_rate < 0.8 || timeout_rate >= 0.2 {
        ProviderHealth::Degraded
    } else {
        ProviderHealth::Healthy
    }
}

fn prune_memory(observations: &mut VecDeque<TelemetryObservation>, cutoff: i64) {
    observations.retain(|item| item.observed_at >= cutoff);
}

pub fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs() as i64)
}

pub(crate) fn category_name(category: &Category) -> &'static str {
    match category {
        Category::General => "general",
        Category::Technical => "technical",
        Category::Code => "code",
        Category::Documentation => "documentation",
        Category::News => "news",
        Category::Academic => "academic",
        Category::Commercial => "commercial",
        Category::Forum => "forum",
        Category::Social => "social",
        Category::Media => "media",
        Category::Navigation => "navigation",
    }
}

fn parse_category(value: &str) -> Option<Category> {
    Some(match value {
        "general" => Category::General,
        "technical" => Category::Technical,
        "code" => Category::Code,
        "documentation" => Category::Documentation,
        "news" => Category::News,
        "academic" => Category::Academic,
        "commercial" => Category::Commercial,
        "forum" => Category::Forum,
        "social" => Category::Social,
        "media" => Category::Media,
        "navigation" => Category::Navigation,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn observation(at: i64, outcome: TelemetryOutcome) -> TelemetryObservation {
        TelemetryObservation {
            observed_at: at,
            provider: "p".into(),
            category: Category::General,
            outcome,
            latency_ms: 100,
            total_results: 10,
            unique_results: 8,
            duplicate_ratio: 0.2,
            top_k_contribution: 0.5,
            diversity: 0.6,
            cost_units: 1,
            request_id: None,
        }
    }

    #[test]
    fn state_thresholds_are_exact() {
        for (sample, expected) in [
            (99, ProviderValueState::Bootstrap),
            (100, ProviderValueState::Learning),
            (500, ProviderValueState::Mature),
        ] {
            let observations = (0..sample)
                .map(|_| observation(1_000, TelemetryOutcome::Success))
                .collect::<Vec<_>>();
            assert_eq!(
                calculate_snapshot("p", None, &observations, 1_000, 30).state,
                expected
            );
        }
    }

    #[test]
    fn older_observations_decay_without_changing_raw_sample() {
        let observations = vec![
            observation(1_000, TelemetryOutcome::Success),
            observation(1_000 - 30 * 86_400, TelemetryOutcome::Timeout),
        ];
        let snapshot = calculate_snapshot("p", None, &observations, 1_000, 30);
        assert_eq!(snapshot.sample, 2);
        assert!((snapshot.weighted_sample - 1.5).abs() < 1e-12);
        assert!(snapshot.success_rate > snapshot.timeout_rate);
    }

    /// A panic while another thread holds `state` poisons the mutex. Memory
    /// recording must survive that (matching the recovery pattern already
    /// used for the client-cache mutex in `fetch.rs`), rather than going
    /// permanently blind for the rest of the process while `storage` keeps
    /// receiving observations — the divergence the audit flagged.
    #[tokio::test]
    async fn survives_a_poisoned_state_mutex_without_losing_future_observations() {
        let telemetry = InMemoryTelemetry::new();
        let poisoned_state = telemetry.state.clone();
        let _ = std::thread::spawn(move || {
            let _guard = poisoned_state.lock().unwrap();
            panic!("simulated panic while holding the telemetry state lock");
        })
        .join();
        assert!(telemetry.state.is_poisoned());

        telemetry
            .record(observation(1_000, TelemetryOutcome::Success))
            .await;

        assert_eq!(telemetry.status().in_memory_observations, 1);
        assert_eq!(telemetry.snapshot_global("p", 1_000).sample, 1);
    }
}
