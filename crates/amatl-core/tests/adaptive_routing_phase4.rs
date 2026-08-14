use amatl_core::{
    parse_query, AdaptiveRouter, Budget, CanonicalUrl, Category, DiversityMetrics,
    InMemoryTelemetry, OriginalUrl, ProgressiveRoundTrace, Provider, ProviderCapabilities,
    ProviderContext, ProviderDescriptor, ProviderError, ProviderExecutionStatus, ProviderItem,
    ProviderResult, Rank, ResultStatus, SearchOrchestrator, SearchPolicyV1, SearchResult,
    SearchStopReason, TelemetryObservation, TelemetryOutcome, SCHEMA_VERSION,
};
use async_trait::async_trait;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use url::Url;

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
                index % domains,
                namespace
            ))
        })
        .collect()
}

fn search_results(count: usize, domains: usize) -> Vec<SearchResult> {
    (0..count)
        .map(|index| {
            let url = Url::parse(&format!(
                "https://d{}.coverage.example/result/{index}",
                index % domains
            ))
            .unwrap();
            SearchResult {
                schema_version: SCHEMA_VERSION.into(),
                rank: Rank::new(index as u32 + 1).unwrap(),
                title: Some("result".into()),
                original_url: OriginalUrl(url.clone()),
                canonical_url: CanonicalUrl(url),
                domain: format!("d{}.coverage.example", index % domains),
                snippet: None,
                providers: vec![format!("p{}", index % 2)],
                published_at: None,
                status: ResultStatus::Visible,
            }
        })
        .collect()
}

struct TestProvider {
    name: String,
    results: Vec<ProviderItem>,
    accepted_filters: Vec<String>,
    delay_ms: u64,
    attempts: AtomicUsize,
}

impl TestProvider {
    fn new(name: &str, results: Vec<ProviderItem>) -> Arc<Self> {
        Arc::new(Self {
            name: name.into(),
            results,
            accepted_filters: vec![],
            delay_ms: 0,
            attempts: AtomicUsize::new(0),
        })
    }

    fn with_filter(name: &str, results: Vec<ProviderItem>, filter: &str) -> Arc<Self> {
        Arc::new(Self {
            name: name.into(),
            results,
            accepted_filters: vec![filter.into()],
            delay_ms: 0,
            attempts: AtomicUsize::new(0),
        })
    }

    fn delayed(name: &str, results: Vec<ProviderItem>, delay_ms: u64) -> Arc<Self> {
        Arc::new(Self {
            name: name.into(),
            results,
            accepted_filters: vec![],
            delay_ms,
            attempts: AtomicUsize::new(0),
        })
    }

    fn attempts(&self) -> usize {
        self.attempts.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl Provider for TestProvider {
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

    async fn search(
        &self,
        _plan: &amatl_core::SearchPlan,
        _context: &ProviderContext,
    ) -> Result<ProviderResult, ProviderError> {
        self.attempts.fetch_add(1, Ordering::SeqCst);
        if self.delay_ms > 0 {
            tokio::time::sleep(Duration::from_millis(self.delay_ms)).await;
        }
        Ok(ProviderResult {
            schema_version: SCHEMA_VERSION.into(),
            provider: self.name.clone(),
            status: ProviderExecutionStatus::Success,
            results: self.results.clone(),
            accepted_filters: self.accepted_filters.clone(),
            ignored_filters: vec![],
            approximated_filters: vec![],
            errors: vec![],
        })
    }
}

fn stop(trace: &[ProgressiveRoundTrace]) -> Option<SearchStopReason> {
    trace.last().and_then(|round| round.stop_reason.clone())
}

#[test]
fn c02_eight_useful_results_and_four_domains_reach_minimum_coverage() {
    let results = search_results(8, 4);
    let metrics = amatl_core::progressive::evaluate_coverage(
        &results,
        &DiversityMetrics {
            visible_results: 8,
            unique_domains: 4,
            unique_providers: 2,
            unique_result_types: 2,
        },
        &SearchPolicyV1::default(),
    );
    assert!(metrics.coverage_minimum);
    assert!(!metrics.coverage_target);
}

#[tokio::test]
async fn c03_target_coverage_stops_before_optional_provider() {
    let a = TestProvider::new("a", items("a", 6, 3));
    let b = TestProvider::new("b", items("b", 6, 3));
    let optional = TestProvider::new("optional", items("optional", 2, 2));
    let providers: Vec<Arc<dyn Provider>> = vec![a, b, optional.clone()];
    let mut orchestrator = SearchOrchestrator::new(Budget::new(3, 5_000), 500);
    orchestrator
        .search(parse_query("rust".into()).unwrap(), providers)
        .await;
    assert_eq!(optional.attempts(), 0);
    assert_eq!(
        stop(orchestrator.routing_trace()),
        Some(SearchStopReason::CoverageTargetReached)
    );
}

#[test]
fn c04_many_visible_results_from_one_domain_are_low_diversity() {
    let results = search_results(12, 1);
    let metrics = amatl_core::progressive::evaluate_coverage(
        &results,
        &DiversityMetrics {
            visible_results: 12,
            unique_domains: 1,
            unique_providers: 2,
            unique_result_types: 1,
        },
        &SearchPolicyV1::default(),
    );
    assert!(metrics.low_diversity);
}

#[test]
fn c05_two_visible_results_do_not_expand_on_diversity_alone() {
    let results = search_results(2, 1);
    let metrics = amatl_core::progressive::evaluate_coverage(
        &results,
        &DiversityMetrics {
            visible_results: 2,
            unique_domains: 1,
            unique_providers: 1,
            unique_result_types: 1,
        },
        &SearchPolicyV1::default(),
    );
    assert!(!metrics.low_diversity);
}

#[tokio::test]
async fn c06_incomplete_coverage_allows_exactly_one_low_gain_exception() {
    let a = TestProvider::new("a", items("a", 1, 1));
    let b = TestProvider::new("b", items("b", 1, 1));
    let exception = TestProvider::new("exception", items("exception", 1, 1));
    let fourth = TestProvider::new("fourth", items("fourth", 1, 1));
    let providers: Vec<Arc<dyn Provider>> = vec![a, b, exception.clone(), fourth.clone()];
    let mut orchestrator = SearchOrchestrator::new(Budget::new(4, 5_000), 500);
    orchestrator
        .search(parse_query("rust".into()).unwrap(), providers)
        .await;
    assert_eq!(exception.attempts(), 1);
    assert_eq!(fourth.attempts(), 0);
    assert_eq!(
        orchestrator.routing_trace()[0].debug_reasons.last(),
        Some(&"coverage_exception_once".into())
    );
    assert_eq!(
        stop(orchestrator.routing_trace()),
        Some(SearchStopReason::MarginalGainLow)
    );
}

#[tokio::test]
async fn c07_minimum_coverage_and_low_expected_gain_stop_expansion() {
    let a = TestProvider::new("a", items("a", 4, 2));
    let b = TestProvider::new("b", items("b", 4, 2));
    let low = TestProvider::new("low", items("low", 2, 2));
    let providers: Vec<Arc<dyn Provider>> = vec![a, b, low.clone()];
    let mut orchestrator = SearchOrchestrator::new(Budget::new(3, 5_000), 500);
    orchestrator
        .search(parse_query("rust".into()).unwrap(), providers)
        .await;
    assert_eq!(low.attempts(), 0);
    assert_eq!(
        stop(orchestrator.routing_trace()),
        Some(SearchStopReason::MarginalGainLow)
    );
}

#[tokio::test]
async fn c08_pending_explicit_filter_allows_capable_provider() {
    let a = TestProvider::new("a", items("a", 6, 3));
    let b = TestProvider::new("b", items("b", 6, 3));
    let filter = TestProvider::with_filter("filter", items("filter", 1, 1), "site");
    let providers: Vec<Arc<dyn Provider>> = vec![a, b, filter.clone()];
    let mut orchestrator = SearchOrchestrator::new(Budget::new(3, 5_000), 500);
    orchestrator
        .search(parse_query("rust site:docs.rs".into()).unwrap(), providers)
        .await;
    assert_eq!(filter.attempts(), 1);
    assert_eq!(
        stop(orchestrator.routing_trace()),
        Some(SearchStopReason::CoverageTargetReached)
    );
}

#[tokio::test]
async fn c09_deadline_near_prevents_a_new_provider() {
    let a = TestProvider::delayed("a", items("a", 1, 1), 100);
    let b = TestProvider::delayed("b", items("b", 1, 1), 100);
    let third = TestProvider::new("third", items("third", 4, 4));
    let providers: Vec<Arc<dyn Provider>> = vec![a, b, third.clone()];
    let mut orchestrator = SearchOrchestrator::new(Budget::new(3, 800), 500);
    orchestrator
        .search(parse_query("rust".into()).unwrap(), providers)
        .await;
    assert_eq!(third.attempts(), 0);
    assert_eq!(
        stop(orchestrator.routing_trace()),
        Some(SearchStopReason::DeadlineNear)
    );
}

#[tokio::test]
async fn c10_identical_inputs_produce_identical_routing_trace() {
    async fn run() -> Vec<ProgressiveRoundTrace> {
        let providers: Vec<Arc<dyn Provider>> = vec![
            TestProvider::new("a", items("a", 1, 1)),
            TestProvider::new("b", items("b", 1, 1)),
            TestProvider::new("c", items("c", 1, 1)),
        ];
        let mut orchestrator = SearchOrchestrator::new(Budget::new(3, 5_000), 500);
        orchestrator
            .search(parse_query("rust".into()).unwrap(), providers)
            .await;
        orchestrator.routing_trace().to_vec()
    }
    assert_eq!(run().await, run().await);
}

#[tokio::test]
async fn new_provider_receives_minimum_exploration_priority() {
    let telemetry = InMemoryTelemetry::new();
    for provider in ["mature-a", "mature-b"] {
        for _ in 0..100 {
            telemetry
                .record(TelemetryObservation {
                    observed_at: amatl_core::telemetry::now_unix(),
                    provider: provider.into(),
                    category: Category::General,
                    outcome: TelemetryOutcome::Success,
                    latency_ms: 10,
                    total_results: 1,
                    unique_results: 1,
                    duplicate_ratio: 0.0,
                    top_k_contribution: 1.0,
                    diversity: 1.0,
                    cost_units: 0,
                    request_id: None,
                })
                .await;
        }
    }
    let query = parse_query("rust".into()).unwrap();
    let capabilities = TestProvider::new("caps", vec![]).capabilities();
    let recommendation = AdaptiveRouter.recommend(
        &query,
        &amatl_core::classify(&query),
        &[
            ProviderDescriptor {
                name: "mature-a".into(),
                capabilities: capabilities.clone(),
                available: true,
            },
            ProviderDescriptor {
                name: "mature-b".into(),
                capabilities: capabilities.clone(),
                available: true,
            },
            ProviderDescriptor {
                name: "new".into(),
                capabilities,
                available: true,
            },
        ],
        &telemetry,
        &SearchPolicyV1::default(),
        amatl_core::telemetry::now_unix(),
    );
    assert!(recommendation.first_round_providers.contains(&"new".into()));
    assert!(recommendation
        .debug_reasons
        .contains(&"minimum_exploration:new".into()));
}
