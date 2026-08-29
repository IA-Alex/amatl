use crate::model::{Category, Classification, ProviderCapabilities, Query};
use crate::progressive::SearchPolicyV1;
use crate::telemetry::{
    InMemoryTelemetry, ProviderHealth, ProviderValueSnapshot, ProviderValueState,
};
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub struct ProviderDescriptor {
    pub name: String,
    pub capabilities: ProviderCapabilities,
    pub available: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoutingRecommendation {
    pub selected_providers: Vec<String>,
    pub provider_budget_requests: BTreeMap<String, u32>,
    pub debug_reasons: Vec<String>,
}

/// Role a provider plays under the primary/expansion model (STEP 1).
///
/// This is deliberately a small, closed enum: the role is decided *before*
/// adaptive scoring and dominates it. STEP 2 (the complementarity contract)
/// will derive FOUND_PRIMARY / FOUND_EXPANSION / OVERLAP from per-result
/// provenance, and it needs to know unambiguously which role produced each
/// result — hence the role travels on the recommendation and the round trace,
/// not just as an ordering side effect.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProviderRole {
    Primary,
    Expansion,
}

impl ProviderRole {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Primary => "primary",
            Self::Expansion => "expansion",
        }
    }
}

/// Declarative role assignment handed to [`AdaptiveRouter::recommend`].
///
/// Built from `[expansion]` config. When `active` is false the router keeps
/// its pre-STEP-1 behavior exactly (legacy mode); the names are still carried
/// so callers and traces can report them, but nothing consults them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RoleAssignment {
    pub active: bool,
    pub primary_provider: String,
    pub expansion_providers: Vec<String>,
}

impl RoleAssignment {
    /// Legacy assignment: role model disabled.
    pub fn legacy() -> Self {
        Self::default()
    }

    /// Active primary/expansion assignment.
    pub fn primary_expansion(
        primary: impl Into<String>,
        expansion: impl IntoIterator<Item = String>,
    ) -> Self {
        Self {
            active: true,
            primary_provider: primary.into(),
            expansion_providers: expansion.into_iter().collect(),
        }
    }

    fn role_of(&self, name: &str) -> Option<ProviderRole> {
        if !self.active {
            return None;
        }
        if name == self.primary_provider {
            Some(ProviderRole::Primary)
        } else if self.expansion_providers.iter().any(|p| p == name) {
            Some(ProviderRole::Expansion)
        } else {
            None
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct AdaptiveRoutingRecommendation {
    pub ordered_providers: Vec<String>,
    pub first_round_providers: Vec<String>,
    pub provider_budget_requests: BTreeMap<String, u32>,
    pub expected_marginal_gain_by_provider: BTreeMap<String, f64>,
    pub provider_states: BTreeMap<String, ProviderValueState>,
    pub excluded_providers: BTreeMap<String, String>,
    pub fallback: bool,
    /// Role the router assigned to each candidate provider. Empty in legacy
    /// mode. Present for every provider in `ordered_providers` when the role
    /// model is active.
    pub provider_roles: BTreeMap<String, ProviderRole>,
    /// The provider that holds the PRIMARY role *and is eligible* this run.
    /// `None` in legacy mode, or when the configured primary is unavailable /
    /// filtered out — in which case expansion providers may still run, but the
    /// result is not a complete PRIMARY search (STEP 1E). Callers record the
    /// distinction in the routing trace and degradations.
    pub primary_provider: Option<String>,
    /// Eligible expansion providers, in adaptive-score order. Empty in legacy
    /// mode.
    pub expansion_providers: Vec<String>,
    pub debug_reasons: Vec<String>,
}

#[derive(Default)]
pub struct AdaptiveRouter;

impl AdaptiveRouter {
    pub fn recommend(
        &self,
        query: &Query,
        classification: &Classification,
        providers: &[ProviderDescriptor],
        telemetry: &InMemoryTelemetry,
        policy: &SearchPolicyV1,
        now: i64,
    ) -> AdaptiveRoutingRecommendation {
        self.recommend_with_roles(
            query,
            classification,
            providers,
            telemetry,
            policy,
            &RoleAssignment::legacy(),
            now,
        )
    }

    /// Like [`Self::recommend`], but with an explicit primary/expansion role
    /// assignment. When `roles.active` is false this is byte-for-byte
    /// equivalent to `recommend`.
    ///
    /// The role dominates the score: after the adaptive ranking produces its
    /// score order, that order is *re-sorted by role* — PRIMARY first (if
    /// eligible), then EXPANSION candidates in their adaptive-score order.
    /// The adaptive score therefore only ever orders providers *within* the
    /// EXPANSION tier; it can never lift an expansion provider ahead of the
    /// primary. See [`ProviderRole`].
    #[allow(clippy::too_many_arguments)]
    pub fn recommend_with_roles(
        &self,
        query: &Query,
        classification: &Classification,
        providers: &[ProviderDescriptor],
        telemetry: &InMemoryTelemetry,
        policy: &SearchPolicyV1,
        roles: &RoleAssignment,
        now: i64,
    ) -> AdaptiveRoutingRecommendation {
        let default_policy;
        let policy = if policy.validate().is_ok() {
            policy
        } else {
            default_policy = SearchPolicyV1::default();
            &default_policy
        };
        let mut excluded = BTreeMap::new();
        let mut eligible = Vec::new();
        for (index, provider) in providers.iter().enumerate() {
            if !provider.available {
                excluded.insert(provider.name.clone(), "provider_unavailable".into());
                continue;
            }
            if !supports_required_filters(query, &provider.capabilities) {
                excluded.insert(provider.name.clone(), "required_capability_missing".into());
                continue;
            }
            let snapshot = telemetry.snapshot_for_routing(
                &provider.name,
                classification.primary_category.clone(),
                now,
            );
            if snapshot.sample > 0 && snapshot.health == ProviderHealth::Unavailable {
                excluded.insert(provider.name.clone(), "provider_health_unavailable".into());
                continue;
            }
            eligible.push((index, provider, snapshot));
        }

        let total_samples = eligible
            .iter()
            .map(|(_, _, snapshot)| snapshot.sample)
            .sum::<u64>();
        let fallback = eligible
            .iter()
            .all(|(_, _, snapshot)| snapshot.state == ProviderValueState::Bootstrap);
        let mut scored = eligible
            .into_iter()
            .map(|(index, provider, snapshot)| {
                let expected_gain = expected_marginal_gain(&snapshot);
                let exploration_due = total_samples > 0
                    && (snapshot.sample == 0
                        || snapshot.sample as f64 / (total_samples as f64)
                            < policy.minimum_exploration_ratio);
                let base = providers.len().saturating_sub(index) as f64 * 0.01
                    + capability_relevance(
                        &classification.primary_category,
                        &provider.capabilities,
                    );
                let adaptive_weight = match snapshot.state {
                    ProviderValueState::Bootstrap => 0.0,
                    ProviderValueState::Learning => 0.25,
                    ProviderValueState::Mature => 0.75,
                };
                let health_penalty = if snapshot.health == ProviderHealth::Degraded {
                    0.5
                } else {
                    0.0
                };
                let exploration_boost = if exploration_due { 2.0 } else { 0.0 };
                let cost_penalty = provider.capabilities.estimated_cost.unwrap_or(0) as f64 * 0.05;
                let score = base + expected_gain * adaptive_weight + exploration_boost
                    - health_penalty
                    - cost_penalty;
                (
                    index,
                    provider.name.clone(),
                    snapshot,
                    expected_gain,
                    exploration_due,
                    score,
                )
            })
            .collect::<Vec<_>>();
        scored.sort_by(|left, right| {
            right
                .5
                .total_cmp(&left.5)
                .then_with(|| left.0.cmp(&right.0))
                .then_with(|| left.1.cmp(&right.1))
        });

        let score_ordered = scored
            .iter()
            .map(|(_, name, _, _, _, _)| name.clone())
            .collect::<Vec<_>>();

        // The role dominates the score. `score_ordered` is the pure adaptive
        // ranking; when the role model is active we re-partition it into
        // PRIMARY (at most one, and only if it survived eligibility) followed
        // by EXPANSION candidates *kept in their adaptive-score order*. A
        // provider with no configured role keeps its score position after the
        // known roles — this only arises transiently while a config change is
        // rolling out, and never reorders relative to the primary.
        let (ordered_providers, provider_roles, primary_provider, expansion_providers) =
            if roles.active {
                let mut primary = Vec::new();
                let mut expansion = Vec::new();
                let mut unroled = Vec::new();
                let mut role_map = BTreeMap::new();
                for name in &score_ordered {
                    match roles.role_of(name) {
                        Some(ProviderRole::Primary) => {
                            role_map.insert(name.clone(), ProviderRole::Primary);
                            primary.push(name.clone());
                        }
                        Some(ProviderRole::Expansion) => {
                            role_map.insert(name.clone(), ProviderRole::Expansion);
                            expansion.push(name.clone());
                        }
                        None => unroled.push(name.clone()),
                    }
                }
                let primary_eligible = primary.first().cloned();
                let mut ordered = Vec::with_capacity(score_ordered.len());
                ordered.extend(primary.iter().cloned());
                ordered.extend(expansion.iter().cloned());
                ordered.extend(unroled.iter().cloned());
                (ordered, role_map, primary_eligible, expansion)
            } else {
                (score_ordered.clone(), BTreeMap::new(), None, Vec::new())
            };

        let first_round_providers = if roles.active {
            // STEP 1C: under the role model round one is PRIMARY only, when
            // the primary is eligible. If the primary is not eligible
            // (STEP 1E) fall back to the first eligible provider so the
            // search can still run — the trace records the missing primary.
            match &primary_provider {
                Some(primary) => vec![primary.clone()],
                None => ordered_providers.iter().take(1).cloned().collect(),
            }
        } else {
            let first_round_count = ordered_providers
                .len()
                .min(policy.first_round_min_providers)
                .min(policy.first_round_max_providers);
            ordered_providers[..first_round_count].to_vec()
        };
        let provider_budget_requests = ordered_providers
            .iter()
            .map(|provider| (provider.clone(), 1))
            .collect();
        let expected_marginal_gain_by_provider = scored
            .iter()
            .map(|(_, name, _, expected, _, _)| (name.clone(), *expected))
            .collect();
        let provider_states = scored
            .iter()
            .map(|(_, name, snapshot, _, _, _)| (name.clone(), snapshot.state.clone()))
            .collect();
        let mut debug_reasons = vec![if fallback {
            "bootstrap_static_routing".into()
        } else {
            "provider_value_adaptive_routing".into()
        }];
        if roles.active {
            match &primary_provider {
                Some(primary) => debug_reasons.push(format!("primary_provider:{primary}")),
                None => debug_reasons.push("primary_provider_unavailable".into()),
            }
            if !expansion_providers.is_empty() {
                debug_reasons.push(format!(
                    "expansion_providers:{}",
                    expansion_providers.join(",")
                ));
            }
        }
        debug_reasons.extend(
            scored
                .iter()
                .filter(|(_, _, _, _, exploration, _)| *exploration)
                .map(|(_, name, _, _, _, _)| format!("minimum_exploration:{name}")),
        );
        AdaptiveRoutingRecommendation {
            ordered_providers,
            first_round_providers,
            provider_budget_requests,
            expected_marginal_gain_by_provider,
            provider_states,
            excluded_providers: excluded,
            fallback,
            provider_roles,
            primary_provider,
            expansion_providers,
            debug_reasons,
        }
    }
}

fn supports_required_filters(query: &Query, capabilities: &ProviderCapabilities) -> bool {
    (query.domains.is_empty() || capabilities.site_filter)
        && (query.file_types.is_empty() || capabilities.file_filter)
        && (query.language.is_none() || capabilities.language)
        && (query.region.is_none() || capabilities.region)
        && (query.date_from.is_none() && query.date_to.is_none() || capabilities.time_range)
}

fn capability_relevance(category: &Category, capabilities: &ProviderCapabilities) -> f64 {
    let relevant = match category {
        Category::Code => capabilities.code,
        Category::Documentation => capabilities.docs,
        Category::News => capabilities.news,
        Category::Academic => capabilities.academic,
        _ => false,
    };
    if relevant {
        1.0
    } else {
        0.0
    }
}

fn expected_marginal_gain(snapshot: &ProviderValueSnapshot) -> f64 {
    if snapshot.state == ProviderValueState::Bootstrap {
        return 0.0;
    }
    let useful_yield = snapshot.average_unique_results * (1.0 - snapshot.duplicate_ratio);
    let bounded_yield = useful_yield / (1.0 + useful_yield);
    let quality = (snapshot.top_k_contribution + snapshot.diversity + snapshot.success_rate) / 3.0;
    let timeout_penalty = (1.0 - snapshot.timeout_rate).powi(2);
    let latency_factor = 1.0 / (1.0 + snapshot.average_latency_ms / 3_000.0);
    let cost_factor = 1.0 / (1.0 + snapshot.average_cost_units);
    (bounded_yield
        * quality
        * snapshot.success_rate
        * timeout_penalty
        * latency_factor
        * cost_factor)
        .max(0.0)
}

#[derive(Default)]
pub struct StaticRouter;

impl StaticRouter {
    pub fn recommend(
        &self,
        query: &Query,
        _classification: &Classification,
        providers: &[ProviderDescriptor],
    ) -> RoutingRecommendation {
        let selected: Vec<_> = providers
            .iter()
            .filter(|provider| provider.available)
            .filter(|provider| {
                (query.domains.is_empty() || provider.capabilities.site_filter)
                    && (query.file_types.is_empty() || provider.capabilities.file_filter)
                    && (query.language.is_none() || provider.capabilities.language)
                    && (query.region.is_none() || provider.capabilities.region)
            })
            .take(3)
            .map(|provider| provider.name.clone())
            .collect();
        let requests = selected.iter().map(|name| (name.clone(), 1)).collect();
        RoutingRecommendation {
            selected_providers: selected,
            provider_budget_requests: requests,
            debug_reasons: vec!["bootstrap_static_routing".into()],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        classify, parse_query, Budget, ProviderCapabilities, TelemetryObservation,
        TelemetryOutcome, SCHEMA_VERSION,
    };
    fn caps(site: bool) -> ProviderCapabilities {
        ProviderCapabilities {
            schema_version: SCHEMA_VERSION.into(),
            pagination: false,
            language: true,
            region: true,
            time_range: false,
            site_filter: site,
            file_filter: true,
            news: false,
            code: false,
            docs: false,
            academic: false,
            authentication: false,
            estimated_cost: None,
        }
    }
    #[test]
    fn router_only_requests_capacity() {
        let query = parse_query("rust site:docs.rs".into()).unwrap();
        let result = StaticRouter.recommend(
            &query,
            &classify(&query),
            &[
                ProviderDescriptor {
                    name: "capable".into(),
                    capabilities: caps(true),
                    available: true,
                },
                ProviderDescriptor {
                    name: "incapable".into(),
                    capabilities: caps(false),
                    available: true,
                },
            ],
        );
        assert_eq!(result.selected_providers, ["capable"]);
        assert_eq!(result.provider_budget_requests["capable"], 1);
    }

    fn descriptor(name: &str) -> ProviderDescriptor {
        ProviderDescriptor {
            name: name.into(),
            capabilities: caps(true),
            available: true,
        }
    }

    async fn record(telemetry: &InMemoryTelemetry, provider: &str, unique_results: u64) {
        for _ in 0..100 {
            telemetry
                .record(TelemetryObservation {
                    observed_at: crate::telemetry::now_unix(),
                    provider: provider.into(),
                    category: Category::General,
                    outcome: TelemetryOutcome::Success,
                    latency_ms: 10,
                    total_results: unique_results,
                    unique_results,
                    duplicate_ratio: 0.0,
                    top_k_contribution: 1.0,
                    diversity: 1.0,
                    cost_units: 0,
                    request_id: None,
                })
                .await;
        }
    }

    #[test]
    fn r01_bootstrap_uses_static_deterministic_order() {
        let query = parse_query("rust".into()).unwrap();
        let result = AdaptiveRouter.recommend(
            &query,
            &classify(&query),
            &[descriptor("a"), descriptor("b"), descriptor("c")],
            &InMemoryTelemetry::new(),
            &SearchPolicyV1::default(),
            crate::telemetry::now_unix(),
        );
        assert!(result.fallback);
        assert_eq!(result.first_round_providers, ["a", "b"]);
    }

    #[tokio::test]
    async fn r02_learning_signal_adjusts_priority_softly() {
        let telemetry = InMemoryTelemetry::new();
        record(&telemetry, "low", 0).await;
        record(&telemetry, "high", 10).await;
        let query = parse_query("rust".into()).unwrap();
        let result = AdaptiveRouter.recommend(
            &query,
            &classify(&query),
            &[descriptor("low"), descriptor("high")],
            &telemetry,
            &SearchPolicyV1::default(),
            crate::telemetry::now_unix(),
        );
        assert!(!result.fallback);
        assert_eq!(result.ordered_providers[0], "high");
    }

    #[test]
    fn r03_unavailable_provider_is_excluded_with_reason() {
        let query = parse_query("rust".into()).unwrap();
        let mut unavailable = descriptor("down");
        unavailable.available = false;
        let result = AdaptiveRouter.recommend(
            &query,
            &classify(&query),
            &[unavailable, descriptor("up")],
            &InMemoryTelemetry::new(),
            &SearchPolicyV1::default(),
            crate::telemetry::now_unix(),
        );
        assert_eq!(result.ordered_providers, ["up"]);
        assert_eq!(result.excluded_providers["down"], "provider_unavailable");
    }

    #[tokio::test]
    async fn r04_sparse_category_uses_weighted_global_view() {
        let telemetry = InMemoryTelemetry::new();
        record(&telemetry, "p", 3).await;
        let snapshot =
            telemetry.snapshot_for_routing("p", Category::Technical, crate::telemetry::now_unix());
        assert_eq!(snapshot.category, Some(Category::Technical));
        assert_eq!(snapshot.sample, 100);
        assert_eq!(snapshot.state, ProviderValueState::Learning);
    }

    #[test]
    fn r05_router_never_mutates_or_reserves_budget() {
        let budget = Budget::new(2, 5_000);
        let before = budget.snapshot();
        let query = parse_query("rust".into()).unwrap();
        let result = AdaptiveRouter.recommend(
            &query,
            &classify(&query),
            &[descriptor("a"), descriptor("b")],
            &InMemoryTelemetry::new(),
            &SearchPolicyV1::default(),
            crate::telemetry::now_unix(),
        );
        assert_eq!(budget.snapshot(), before);
        assert_eq!(result.provider_budget_requests.len(), 2);
    }

    #[test]
    fn legacy_role_assignment_is_byte_for_byte_equivalent_to_recommend() {
        let query = parse_query("rust".into()).unwrap();
        let providers = [descriptor("a"), descriptor("b"), descriptor("c")];
        let telemetry = InMemoryTelemetry::new();
        let policy = SearchPolicyV1::default();
        let now = crate::telemetry::now_unix();
        let plain = AdaptiveRouter.recommend(
            &query,
            &classify(&query),
            &providers,
            &telemetry,
            &policy,
            now,
        );
        let legacy = AdaptiveRouter.recommend_with_roles(
            &query,
            &classify(&query),
            &providers,
            &telemetry,
            &policy,
            &RoleAssignment::legacy(),
            now,
        );
        assert_eq!(plain, legacy);
        assert!(legacy.provider_roles.is_empty());
        assert!(legacy.primary_provider.is_none());
    }

    #[test]
    fn role_dominates_score_and_primary_runs_first_round_alone() {
        let query = parse_query("rust".into()).unwrap();
        // `expansion` is listed first, so a pure index/score order would put it
        // ahead — the role must override that.
        let providers = [descriptor("expansion"), descriptor("primary")];
        let roles = RoleAssignment::primary_expansion("primary", ["expansion".to_string()]);
        let recommendation = AdaptiveRouter.recommend_with_roles(
            &query,
            &classify(&query),
            &providers,
            &InMemoryTelemetry::new(),
            &SearchPolicyV1::default(),
            &roles,
            crate::telemetry::now_unix(),
        );
        assert_eq!(recommendation.ordered_providers, ["primary", "expansion"]);
        assert_eq!(recommendation.first_round_providers, ["primary"]);
        assert_eq!(recommendation.primary_provider.as_deref(), Some("primary"));
        assert_eq!(recommendation.expansion_providers, ["expansion"]);
        assert_eq!(
            recommendation.provider_roles.get("primary"),
            Some(&ProviderRole::Primary)
        );
    }

    #[test]
    fn unavailable_primary_still_lets_expansion_run_but_is_reported() {
        let query = parse_query("rust".into()).unwrap();
        let mut primary = descriptor("primary");
        primary.available = false;
        let providers = [primary, descriptor("expansion")];
        let roles = RoleAssignment::primary_expansion("primary", ["expansion".to_string()]);
        let recommendation = AdaptiveRouter.recommend_with_roles(
            &query,
            &classify(&query),
            &providers,
            &InMemoryTelemetry::new(),
            &SearchPolicyV1::default(),
            &roles,
            crate::telemetry::now_unix(),
        );
        assert!(recommendation.primary_provider.is_none());
        assert_eq!(recommendation.first_round_providers, ["expansion"]);
        assert!(recommendation
            .debug_reasons
            .contains(&"primary_provider_unavailable".to_string()));
    }

    #[test]
    fn r06_same_inputs_are_reproducible_and_query_is_unchanged() {
        let query = parse_query("rust lang:es".into()).unwrap();
        let original = query.clone();
        let providers = [descriptor("a"), descriptor("b"), descriptor("c")];
        let telemetry = InMemoryTelemetry::new();
        let policy = SearchPolicyV1::default();
        let now = crate::telemetry::now_unix();
        let first = AdaptiveRouter.recommend(
            &query,
            &classify(&query),
            &providers,
            &telemetry,
            &policy,
            now,
        );
        let second = AdaptiveRouter.recommend(
            &query,
            &classify(&query),
            &providers,
            &telemetry,
            &policy,
            now,
        );
        assert_eq!(first, second);
        assert_eq!(query, original);
    }
}
