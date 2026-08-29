//! STEP 1 — PRIMARY / EXPANSION provider roles.
//!
//! These tests pin the role model down: MARGINALIA is PRIMARY, SEARXNG is
//! EXPANSION, the role dominates the adaptive score, the primary runs alone in
//! round one, and expansion is only reached on structural triggers. They use
//! mock providers only — no real Marginalia call is needed to validate the
//! routing architecture.

use amatl_core::{
    classify, parse_query, AdaptiveRouter, Budget, Category, InMemoryTelemetry,
    ProgressiveRoundTrace, Provider, ProviderAvailability, ProviderCapabilities, ProviderContext,
    ProviderDescriptor, ProviderError, ProviderErrorKind, ProviderExecutionStatus, ProviderItem,
    ProviderResult, Rank, RoleAssignment, SearchOrchestrator, SearchPolicyV1, TelemetryObservation,
    TelemetryOutcome, SCHEMA_VERSION,
};
use async_trait::async_trait;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

fn item(url: String) -> ProviderItem {
    ProviderItem {
        title: Some("useful result".into()),
        url,
        provider_rank: Some(Rank::new(1).unwrap()),
        snippet: None,
        result_type: None,
        published_at: None,
        author: None,
        language: None,
        file_type: None,
        thumbnail: None,
        metadata: BTreeMap::new(),
    }
}

fn items(namespace: &str, count: usize, domains: usize) -> Vec<ProviderItem> {
    (0..count)
        .map(|index| {
            item(format!(
                "https://d{}.{}.example/result/{index}",
                index % domains.max(1),
                namespace
            ))
        })
        .collect()
}

#[derive(Clone)]
enum Behavior {
    Success(Vec<ProviderItem>),
    Failure(ProviderErrorKind),
}

struct RoleTestProvider {
    name: String,
    behavior: Behavior,
    available: bool,
    attempts: AtomicUsize,
}

impl RoleTestProvider {
    fn success(name: &str, results: Vec<ProviderItem>) -> Arc<Self> {
        Arc::new(Self {
            name: name.into(),
            behavior: Behavior::Success(results),
            available: true,
            attempts: AtomicUsize::new(0),
        })
    }

    fn failing(name: &str, kind: ProviderErrorKind) -> Arc<Self> {
        Arc::new(Self {
            name: name.into(),
            behavior: Behavior::Failure(kind),
            available: true,
            attempts: AtomicUsize::new(0),
        })
    }

    fn unavailable(name: &str) -> Arc<Self> {
        Arc::new(Self {
            name: name.into(),
            behavior: Behavior::Success(vec![]),
            available: false,
            attempts: AtomicUsize::new(0),
        })
    }

    fn attempts(&self) -> usize {
        self.attempts.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl Provider for RoleTestProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            schema_version: SCHEMA_VERSION.into(),
            pagination: true,
            language: true,
            region: true,
            time_range: true,
            site_filter: true,
            file_filter: true,
            news: true,
            code: true,
            docs: true,
            academic: true,
            authentication: false,
            estimated_cost: Some(0),
        }
    }

    fn availability(&self) -> ProviderAvailability {
        if self.available {
            ProviderAvailability::Available
        } else {
            ProviderAvailability::Unavailable {
                code: "provider_unavailable".into(),
                message: "mock unavailable".into(),
            }
        }
    }

    async fn search(
        &self,
        _plan: &amatl_core::SearchPlan,
        _context: &ProviderContext,
    ) -> Result<ProviderResult, ProviderError> {
        self.attempts.fetch_add(1, Ordering::SeqCst);
        match &self.behavior {
            Behavior::Success(results) => Ok(ProviderResult {
                schema_version: SCHEMA_VERSION.into(),
                provider: self.name.clone(),
                status: ProviderExecutionStatus::Success,
                results: results.clone(),
                accepted_filters: vec![],
                ignored_filters: vec![],
                approximated_filters: vec![],
                errors: vec![],
            }),
            Behavior::Failure(kind) => Err(ProviderError {
                schema_version: SCHEMA_VERSION.into(),
                provider: self.name.clone(),
                kind: kind.clone(),
                message: "mock provider failure".into(),
                retry_after_ms: None,
            }),
        }
    }
}

fn marginalia_expansion() -> RoleAssignment {
    RoleAssignment::primary_expansion("marginalia", ["searxng".to_string()])
}

fn round_one(trace: &[ProgressiveRoundTrace]) -> Vec<String> {
    trace
        .first()
        .map(|round| round.providers_selected.clone())
        .unwrap_or_default()
}

/// 1. Marginalia + SearXNG both available: ROUND_1 = Marginalia only.
#[tokio::test]
async fn primary_runs_alone_in_round_one() {
    let marginalia = RoleTestProvider::success("marginalia", items("m", 6, 3));
    let searxng = RoleTestProvider::success("searxng", items("s", 6, 3));
    let providers: Vec<Arc<dyn Provider>> = vec![marginalia.clone(), searxng.clone()];
    let mut orchestrator = SearchOrchestrator::new(Budget::new(3, 5_000), 500)
        .with_role_assignment(marginalia_expansion());
    orchestrator
        .search(parse_query("rust".into()).unwrap(), providers)
        .await;
    assert_eq!(round_one(orchestrator.routing_trace()), ["marginalia"]);
    assert_eq!(marginalia.attempts(), 1);
}

/// 2. SearXNG has a higher adaptive score, PRIMARY is still Marginalia.
#[tokio::test]
async fn expansion_does_not_override_primary_by_score() {
    let telemetry = InMemoryTelemetry::new();
    for _ in 0..200 {
        telemetry
            .record(TelemetryObservation {
                observed_at: amatl_core::telemetry::now_unix(),
                provider: "searxng".into(),
                category: Category::General,
                outcome: TelemetryOutcome::Success,
                latency_ms: 5,
                total_results: 20,
                unique_results: 20,
                duplicate_ratio: 0.0,
                top_k_contribution: 1.0,
                diversity: 1.0,
                cost_units: 0,
                request_id: None,
            })
            .await;
    }
    for _ in 0..200 {
        telemetry
            .record(TelemetryObservation {
                observed_at: amatl_core::telemetry::now_unix(),
                provider: "marginalia".into(),
                category: Category::General,
                outcome: TelemetryOutcome::Success,
                latency_ms: 500,
                total_results: 1,
                unique_results: 0,
                duplicate_ratio: 1.0,
                top_k_contribution: 0.0,
                diversity: 0.0,
                cost_units: 0,
                request_id: None,
            })
            .await;
    }
    let query = parse_query("rust".into()).unwrap();
    let capabilities = RoleTestProvider::success("caps", vec![]).capabilities();
    let recommendation = AdaptiveRouter.recommend_with_roles(
        &query,
        &classify(&query),
        &[
            ProviderDescriptor {
                name: "searxng".into(),
                capabilities: capabilities.clone(),
                available: true,
            },
            ProviderDescriptor {
                name: "marginalia".into(),
                capabilities,
                available: true,
            },
        ],
        &telemetry,
        &SearchPolicyV1::default(),
        &marginalia_expansion(),
        amatl_core::telemetry::now_unix(),
    );
    assert_eq!(
        recommendation.primary_provider.as_deref(),
        Some("marginalia")
    );
    assert_eq!(recommendation.first_round_providers, ["marginalia"]);
    assert_eq!(recommendation.ordered_providers[0], "marginalia");
    assert_eq!(recommendation.expansion_providers, ["searxng"]);
}

/// 3. Marginalia fails: SearXNG can run a later round.
#[tokio::test]
async fn primary_failure_triggers_expansion() {
    let marginalia = RoleTestProvider::failing("marginalia", ProviderErrorKind::Network);
    let searxng = RoleTestProvider::success("searxng", items("s", 6, 3));
    let providers: Vec<Arc<dyn Provider>> = vec![marginalia.clone(), searxng.clone()];
    let mut orchestrator = SearchOrchestrator::new(Budget::new(3, 5_000), 500)
        .with_role_assignment(marginalia_expansion());
    orchestrator
        .search(parse_query("rust".into()).unwrap(), providers)
        .await;
    assert_eq!(round_one(orchestrator.routing_trace()), ["marginalia"]);
    assert!(
        searxng.attempts() >= 1,
        "expansion must run after primary failure"
    );
}

/// 4. Marginalia rate-limited (429): SearXNG is used as expansion / fallback.
#[tokio::test]
async fn primary_rate_limit_triggers_expansion() {
    let marginalia = RoleTestProvider::failing("marginalia", ProviderErrorKind::RateLimit);
    let searxng = RoleTestProvider::success("searxng", items("s", 6, 3));
    let providers: Vec<Arc<dyn Provider>> = vec![marginalia.clone(), searxng.clone()];
    let mut orchestrator = SearchOrchestrator::new(Budget::new(3, 5_000), 500)
        .with_role_assignment(marginalia_expansion());
    orchestrator
        .search(parse_query("rust".into()).unwrap(), providers)
        .await;
    assert!(
        searxng.attempts() >= 1,
        "expansion must run after primary rate limit"
    );
}

/// 5. Marginalia success + zero results: SearXNG runs.
#[tokio::test]
async fn primary_zero_results_triggers_expansion() {
    let marginalia = RoleTestProvider::success("marginalia", vec![]);
    let searxng = RoleTestProvider::success("searxng", items("s", 6, 3));
    let providers: Vec<Arc<dyn Provider>> = vec![marginalia.clone(), searxng.clone()];
    let mut orchestrator = SearchOrchestrator::new(Budget::new(3, 5_000), 500)
        .with_role_assignment(marginalia_expansion());
    orchestrator
        .search(parse_query("rust".into()).unwrap(), providers)
        .await;
    assert_eq!(round_one(orchestrator.routing_trace()), ["marginalia"]);
    assert!(
        searxng.attempts() >= 1,
        "expansion must run when primary returns nothing"
    );
}

/// 6. Marginalia produces some results but not enough for coverage_minimum:
///    SearXNG runs.
#[tokio::test]
async fn insufficient_primary_coverage_triggers_expansion() {
    // Two results / one domain is well below the default minimum (8 / 4).
    let marginalia = RoleTestProvider::success("marginalia", items("m", 2, 1));
    let searxng = RoleTestProvider::success("searxng", items("s", 6, 3));
    let providers: Vec<Arc<dyn Provider>> = vec![marginalia.clone(), searxng.clone()];
    let mut orchestrator = SearchOrchestrator::new(Budget::new(3, 5_000), 500)
        .with_role_assignment(marginalia_expansion());
    orchestrator
        .search(parse_query("rust".into()).unwrap(), providers)
        .await;
    assert!(
        searxng.attempts() >= 1,
        "expansion must run when primary coverage is insufficient"
    );
}

/// 7. Marginalia reaches sufficient coverage: SearXNG does NOT run just to
///    satisfy a legacy first-round minimum.
#[tokio::test]
async fn sufficient_primary_coverage_does_not_force_expansion() {
    let marginalia = RoleTestProvider::success("marginalia", items("m", 14, 7));
    let searxng = RoleTestProvider::success("searxng", items("s", 6, 3));
    let providers: Vec<Arc<dyn Provider>> = vec![marginalia.clone(), searxng.clone()];
    let mut orchestrator = SearchOrchestrator::new(Budget::new(3, 5_000), 500)
        .with_role_assignment(marginalia_expansion());
    orchestrator
        .search(parse_query("rust".into()).unwrap(), providers)
        .await;
    assert_eq!(round_one(orchestrator.routing_trace()), ["marginalia"]);
    assert_eq!(
        searxng.attempts(),
        0,
        "expansion must not run once primary coverage is sufficient"
    );
}

/// 8. Marginalia unavailable + SearXNG available: the search still runs, but
///    the routing trace evidences the missing PRIMARY.
#[tokio::test]
async fn only_expansion_available_is_traceable() {
    let marginalia = RoleTestProvider::unavailable("marginalia");
    let searxng = RoleTestProvider::success("searxng", items("s", 6, 3));
    let providers: Vec<Arc<dyn Provider>> = vec![marginalia.clone(), searxng.clone()];
    let mut orchestrator = SearchOrchestrator::new(Budget::new(3, 5_000), 500)
        .with_role_assignment(marginalia_expansion());
    let response = orchestrator
        .search(parse_query("rust".into()).unwrap(), providers)
        .await;
    assert_eq!(marginalia.attempts(), 0);
    assert!(searxng.attempts() >= 1);
    let first = &orchestrator.routing_trace()[0];
    assert_eq!(first.primary_available, Some(false));
    assert!(response
        .degradations
        .iter()
        .any(|degradation| degradation.code == "primary_provider_unavailable"));
    assert!(response.providers_used.contains(&"searxng".to_string()));
}

/// 9. Legacy mode preserves the pre-STEP-1 behavior: both providers run in
///    round one.
#[tokio::test]
async fn legacy_mode_preserves_existing_behavior() {
    let marginalia = RoleTestProvider::success("marginalia", items("m", 3, 2));
    let searxng = RoleTestProvider::success("searxng", items("s", 3, 2));
    let providers: Vec<Arc<dyn Provider>> = vec![marginalia.clone(), searxng.clone()];
    let mut orchestrator = SearchOrchestrator::new(Budget::new(3, 5_000), 500)
        .with_role_assignment(RoleAssignment::legacy());
    orchestrator
        .search(parse_query("rust".into()).unwrap(), providers)
        .await;
    let mut first_round = round_one(orchestrator.routing_trace());
    first_round.sort();
    assert_eq!(first_round, ["marginalia", "searxng"]);
    let first = &orchestrator.routing_trace()[0];
    assert_eq!(first.primary_available, None);
    assert!(first.provider_roles.is_empty());
}

/// 10. Determinism: identical inputs produce an identical role/expansion trace.
#[tokio::test]
async fn identical_inputs_produce_identical_primary_expansion_trace() {
    async fn run() -> Vec<ProgressiveRoundTrace> {
        let providers: Vec<Arc<dyn Provider>> = vec![
            RoleTestProvider::success("marginalia", items("m", 2, 1)),
            RoleTestProvider::success("searxng", items("s", 2, 1)),
        ];
        let mut orchestrator = SearchOrchestrator::new(Budget::new(3, 5_000), 500)
            .with_role_assignment(marginalia_expansion());
        orchestrator
            .search(parse_query("rust".into()).unwrap(), providers)
            .await;
        orchestrator.routing_trace().to_vec()
    }
    let first = run().await;
    let second = run().await;
    assert_eq!(first, second);
    assert_eq!(first[0].providers_selected, ["marginalia"]);
    assert_eq!(
        first[0]
            .provider_roles
            .get("marginalia")
            .map(String::as_str),
        Some("primary")
    );
}
