//! STEP 2 — COMPLEMENTARITY CONTRACT.
//!
//! These tests pin down the objective, structural measurement of what PRIMARY
//! found, what EXPANSION found, what overlapped, and what was exclusive — and
//! pin down, just as hard, that none of it is a relevance claim. Mock providers
//! only; no real Marginalia call.

use amatl_core::{
    compute_complementarity_metrics, parse_query, Budget, CanonicalUrl, DeduplicatedResult,
    DuplicateStatus, OriginalUrl, Provider, ProviderAvailability, ProviderCapabilities,
    ProviderContext, ProviderError, ProviderExecutionStatus, ProviderItem, ProviderResult, Rank,
    RelevanceAssessmentStatus, RoleAssignment, SearchOrchestrator, SearchResponse, SCHEMA_VERSION,
};
use async_trait::async_trait;
use std::collections::BTreeMap;
use std::sync::Arc;

fn provider_item(url: &str) -> ProviderItem {
    ProviderItem {
        title: Some("a sufficiently long result title here".into()),
        url: url.into(),
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

/// Provider that returns exactly the URLs it is given.
struct UrlProvider {
    name: String,
    urls: Vec<String>,
    available: bool,
}

impl UrlProvider {
    fn new(name: &str, urls: &[&str]) -> Arc<Self> {
        Arc::new(Self {
            name: name.into(),
            urls: urls.iter().map(|u| u.to_string()).collect(),
            available: true,
        })
    }
}

#[async_trait]
impl Provider for UrlProvider {
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
        Ok(ProviderResult {
            schema_version: SCHEMA_VERSION.into(),
            provider: self.name.clone(),
            status: ProviderExecutionStatus::Success,
            results: self.urls.iter().map(|u| provider_item(u)).collect(),
            accepted_filters: vec![],
            ignored_filters: vec![],
            approximated_filters: vec![],
            errors: vec![],
        })
    }
}

fn roles() -> RoleAssignment {
    RoleAssignment::primary_expansion("marginalia", ["searxng".to_string()])
}

async fn run(
    marginalia: &[&str],
    searxng: &[&str],
    role_assignment: RoleAssignment,
) -> SearchResponse {
    let providers: Vec<Arc<dyn Provider>> = vec![
        UrlProvider::new("marginalia", marginalia),
        UrlProvider::new("searxng", searxng),
    ];
    // Small budget for provider calls but generous time; enough to reach the
    // expansion round.
    let mut orchestrator =
        SearchOrchestrator::new(Budget::new(4, 5_000), 500).with_role_assignment(role_assignment);
    orchestrator
        .search(parse_query("rust async".into()).unwrap(), providers)
        .await
}

// --- Direct unit tests of the canonical function --------------------------

fn deduped(url: &str, providers: &[&str], possible_with: &[&str]) -> DeduplicatedResult {
    let parsed = url::Url::parse(url).unwrap();
    DeduplicatedResult {
        schema_version: SCHEMA_VERSION.into(),
        title: Some("a sufficiently long result title here".into()),
        original_url: OriginalUrl(parsed.clone()),
        canonical_url: CanonicalUrl(parsed.clone()),
        original_urls: vec![OriginalUrl(parsed)],
        providers: providers.iter().map(|p| p.to_string()).collect(),
        representative_provider: providers.first().copied().unwrap_or("").into(),
        provider_ranks: BTreeMap::new(),
        snippet: None,
        alternate_snippets: vec![],
        result_type: amatl_core::ResultType::Organic,
        published_at: None,
        author: None,
        language: None,
        file_type: None,
        thumbnail: None,
        metadata: BTreeMap::new(),
        observed_dates: vec![],
        duplicate_status: if !possible_with.is_empty() {
            DuplicateStatus::PossibleDuplicate
        } else if providers.len() > 1 {
            DuplicateStatus::ConfirmedDuplicate
        } else {
            DuplicateStatus::Distinct
        },
        merge_reason: None,
        possible_duplicate_with: possible_with
            .iter()
            .map(|u| CanonicalUrl(url::Url::parse(u).unwrap()))
            .collect(),
    }
}

#[test]
fn primary_only_metrics_are_correct() {
    let set = [
        deduped("https://a.example/1", &["marginalia"], &[]),
        deduped("https://b.example/2", &["marginalia"], &[]),
        deduped("https://c.example/3", &["marginalia"], &[]),
    ];
    let m = compute_complementarity_metrics(&set, &roles()).unwrap();
    assert_eq!(m.found_primary, 3);
    assert_eq!(m.found_expansion, 0);
    assert_eq!(m.overlap_confirmed, 0);
    assert_eq!(m.unique_primary, 3);
    assert_eq!(m.unique_expansion, 0);
    assert_eq!(m.expansion_overlap_ratio, None);
    assert_eq!(m.expansion_unique_ratio, None);
}

#[test]
fn expansion_only_metrics_are_correct() {
    let set = [
        deduped("https://a.example/1", &["searxng"], &[]),
        deduped("https://b.example/2", &["searxng"], &[]),
    ];
    let m = compute_complementarity_metrics(&set, &roles()).unwrap();
    assert_eq!(m.found_primary, 0);
    assert_eq!(m.found_expansion, 2);
    assert_eq!(m.overlap_confirmed, 0);
    assert_eq!(m.unique_primary, 0);
    assert_eq!(m.unique_expansion, 2);
    assert_eq!(m.expansion_overlap_ratio, Some(0.0));
    assert_eq!(m.expansion_unique_ratio, Some(1.0));
}

#[test]
fn no_overlap_metrics_are_correct() {
    let set = [
        deduped("https://a.example/1", &["marginalia"], &[]),
        deduped("https://b.example/2", &["marginalia"], &[]),
        deduped("https://x.example/9", &["searxng"], &[]),
    ];
    let m = compute_complementarity_metrics(&set, &roles()).unwrap();
    assert_eq!(m.overlap_confirmed, 0);
    assert_eq!(m.unique_primary, m.found_primary);
    assert_eq!(m.unique_expansion, m.found_expansion);
    assert_eq!(m.found_primary, 2);
    assert_eq!(m.found_expansion, 1);
}

#[test]
fn full_overlap_metrics_are_correct() {
    let set = [
        deduped("https://a.example/1", &["marginalia", "searxng"], &[]),
        deduped("https://b.example/2", &["marginalia", "searxng"], &[]),
    ];
    let m = compute_complementarity_metrics(&set, &roles()).unwrap();
    assert_eq!(m.overlap_confirmed, m.found_primary);
    assert_eq!(m.overlap_confirmed, m.found_expansion);
    assert_eq!(m.overlap_confirmed, 2);
    assert_eq!(m.unique_primary, 0);
    assert_eq!(m.unique_expansion, 0);
}

#[test]
fn partial_overlap_metrics_are_correct() {
    let set = [
        deduped("https://a.example/1", &["marginalia", "searxng"], &[]),
        deduped("https://b.example/2", &["marginalia"], &[]),
        deduped("https://c.example/3", &["marginalia"], &[]),
        deduped("https://x.example/9", &["searxng"], &[]),
        deduped("https://y.example/8", &["searxng"], &[]),
    ];
    let m = compute_complementarity_metrics(&set, &roles()).unwrap();
    assert_eq!(m.found_primary, 3);
    assert_eq!(m.found_expansion, 3);
    assert_eq!(m.overlap_confirmed, 1);
    assert_eq!(m.unique_primary, 2);
    assert_eq!(m.unique_expansion, 2);
    // exact arithmetic
    assert_eq!(m.found_primary, m.overlap_confirmed + m.unique_primary);
    assert_eq!(m.found_expansion, m.overlap_confirmed + m.unique_expansion);
    assert_eq!(m.expansion_overlap_ratio, Some(1.0 / 3.0));
    assert_eq!(m.expansion_unique_ratio, Some(2.0 / 3.0));
}

#[test]
fn possible_duplicate_is_not_confirmed_overlap() {
    // One PRIMARY-only result and one EXPANSION-only result on different hosts,
    // linked only by title similarity (possible duplicate). It must NOT count
    // as confirmed overlap.
    let set = [
        deduped(
            "https://primary.example/doc",
            &["marginalia"],
            &["https://expansion.example/doc"],
        ),
        deduped(
            "https://expansion.example/doc",
            &["searxng"],
            &["https://primary.example/doc"],
        ),
    ];
    let m = compute_complementarity_metrics(&set, &roles()).unwrap();
    assert_eq!(m.overlap_confirmed, 0);
    assert_eq!(m.overlap_possible, 1);
    assert_eq!(m.unique_primary, 1);
    assert_eq!(m.unique_expansion, 1);
}

#[test]
fn expansion_new_domains_excludes_primary_domains() {
    let set = [
        deduped("https://shared.example/1", &["marginalia", "searxng"], &[]),
        deduped("https://primaryonly.example/2", &["marginalia"], &[]),
        deduped("https://new1.example/3", &["searxng"], &[]),
        deduped("https://new2.example/4", &["searxng"], &[]),
        // expansion result on a host PRIMARY also carried -> not new
        deduped("https://shared.example/5", &["searxng"], &[]),
    ];
    let m = compute_complementarity_metrics(&set, &roles()).unwrap();
    // new1, new2 only
    assert_eq!(m.expansion_new_domains, 2);
    assert_eq!(m.primary_unique_domains, 1); // primaryonly.example
}

#[test]
fn zero_expansion_results_do_not_produce_nan() {
    let set = [
        deduped("https://a.example/1", &["marginalia"], &[]),
        deduped("https://b.example/2", &["marginalia"], &[]),
    ];
    let m = compute_complementarity_metrics(&set, &roles()).unwrap();
    assert!(m.expansion_overlap_ratio.is_none());
    assert!(m.expansion_unique_ratio.is_none());
    // and if we serialize, no NaN sneaks in
    let json = serde_json::to_string(&m).unwrap();
    assert!(!json.contains("NaN"));
}

#[test]
fn multiple_expansion_providers_preserve_provenance() {
    // Two expansion providers both find the same URL; provenance must keep both.
    let mut assignment = RoleAssignment::primary_expansion(
        "marginalia",
        ["searxng".to_string(), "mojeek".to_string()],
    );
    assignment.active = true;
    let set = [
        deduped("https://a.example/1", &["searxng", "mojeek"], &[]),
        deduped("https://b.example/2", &["marginalia"], &[]),
    ];
    let m = compute_complementarity_metrics(&set, &assignment).unwrap();
    assert_eq!(m.found_expansion, 1);
    assert_eq!(m.found_primary, 1);
    assert_eq!(m.unique_expansion, 1);
    assert_eq!(set[0].providers, ["searxng", "mojeek"]);
}

#[test]
fn metrics_are_identical_for_identical_inputs() {
    let build = || {
        [
            deduped("https://a.example/1", &["marginalia", "searxng"], &[]),
            deduped("https://b.example/2", &["marginalia"], &[]),
            deduped("https://x.example/9", &["searxng"], &[]),
        ]
    };
    let first = compute_complementarity_metrics(&build(), &roles()).unwrap();
    let second = compute_complementarity_metrics(&build(), &roles()).unwrap();
    assert_eq!(first, second);
}

#[test]
fn unique_expansion_is_not_marked_relevant() {
    let set = [
        deduped("https://x.example/9", &["searxng"], &[]),
        deduped("https://y.example/8", &["searxng"], &[]),
    ];
    let m = compute_complementarity_metrics(&set, &roles()).unwrap();
    assert_eq!(m.unique_expansion, 2);
    // The relevance layer makes no claim about those two results.
    assert_eq!(
        m.relevance.status,
        RelevanceAssessmentStatus::NotImplemented
    );
    assert_eq!(m.relevance.relevant_expansion, None);
    assert_eq!(m.relevance.unique_relevant_expansion, None);
    assert_eq!(m.relevance.expansion_noise_rate, None);
    // And it is not reachable by name confusion in the serialized form.
    let json = serde_json::to_value(&m).unwrap();
    assert!(json.get("unique_relevant_expansion").is_none());
    assert_eq!(json["unique_expansion"], 2);
    assert_eq!(json["relevance"]["status"], "not_implemented");
}

#[test]
fn legacy_mode_does_not_break_metric_calculation() {
    let set = [
        deduped("https://a.example/1", &["marginalia"], &[]),
        deduped("https://b.example/2", &["searxng"], &[]),
    ];
    // Legacy mode: no roles -> no complementarity, and no panic.
    assert!(compute_complementarity_metrics(&set, &RoleAssignment::legacy()).is_none());
}

// --- End-to-end tests through the orchestrator ---------------------------

#[tokio::test]
async fn round_trace_shows_accumulated_complementarity() {
    // PRIMARY returns too little for coverage -> expansion runs in a later
    // round. Round 1 has EXPANSION = 0; a later round shows EXPANSION > 0, with
    // the metric computed on the accumulated results.
    let providers: Vec<Arc<dyn Provider>> = vec![
        UrlProvider::new(
            "marginalia",
            &["https://p1.example/a", "https://p2.example/b"],
        ),
        UrlProvider::new(
            "searxng",
            &[
                "https://p1.example/a", // overlaps p1 by canonical URL
                "https://s1.example/c",
                "https://s2.example/d",
                "https://s3.example/e",
            ],
        ),
    ];
    let mut orchestrator =
        SearchOrchestrator::new(Budget::new(4, 5_000), 500).with_role_assignment(roles());
    orchestrator
        .search(parse_query("rust async".into()).unwrap(), providers)
        .await;
    let trace = orchestrator.routing_trace();
    assert!(trace.len() >= 2, "expansion round must have run");

    let round1 = trace[0].complementarity.as_ref().unwrap();
    assert_eq!(round1.found_primary, 2);
    assert_eq!(round1.found_expansion, 0);
    assert_eq!(round1.unique_primary, 2);

    let last = trace
        .iter()
        .rev()
        .find_map(|r| r.complementarity.as_ref())
        .unwrap();
    assert_eq!(last.found_expansion, 4);
    assert_eq!(last.overlap_confirmed, 1);
    assert_eq!(last.unique_expansion, 3);
    assert_eq!(last.expansion_new_domains, 3);
}

#[tokio::test]
async fn search_response_exposes_complementarity_additively() {
    let response = run(
        &["https://p1.example/a", "https://p2.example/b"],
        &[
            "https://p1.example/a",
            "https://s1.example/c",
            "https://s2.example/d",
        ],
        roles(),
    )
    .await;
    let complementarity = response
        .complementarity
        .as_ref()
        .expect("role model active -> complementarity present");
    assert!(complementarity.found_primary >= 1);

    // Additive: serializing and re-parsing as a value keeps every legacy field
    // and simply adds `complementarity`.
    let value = serde_json::to_value(&response).unwrap();
    assert!(value.get("results").is_some());
    assert!(value.get("status").is_some());
    assert_eq!(value["schema_version"], SCHEMA_VERSION);
    assert!(value.get("complementarity").is_some());

    // A legacy-mode search omits the field entirely.
    let legacy = run(
        &["https://p1.example/a"],
        &["https://s1.example/c"],
        RoleAssignment::legacy(),
    )
    .await;
    assert!(legacy.complementarity.is_none());
    let legacy_value = serde_json::to_value(&legacy).unwrap();
    assert!(legacy_value.get("complementarity").is_none());
}

#[tokio::test]
async fn round_trace_carries_per_round_complementarity_when_roles_active() {
    let response = run(
        &["https://p1.example/a", "https://p2.example/b"],
        &[
            "https://s1.example/c",
            "https://s2.example/d",
            "https://s3.example/e",
        ],
        roles(),
    )
    .await;
    // Response-level metrics exist; the trace is validated in
    // primary_expansion_roles.rs for determinism. Here just assert the response
    // path is populated and self-consistent.
    let m = response.complementarity.unwrap();
    assert_eq!(m.found_primary, m.overlap_confirmed + m.unique_primary);
    assert_eq!(m.found_expansion, m.overlap_confirmed + m.unique_expansion);
}
