//! STEP 2E — DETERMINISTIC RELEVANCE SIGNAL.
//!
//! Pins down the first deterministic relevance layer and — just as hard — that
//! it is a pure *observation*: it never changes routing, the AdaptiveRouter
//! never consumes it, the raw structural `unique_expansion` count is preserved,
//! and the payload change is additive. Mock providers only; fully offline.

use amatl_core::{
    assess_relevance, assess_result, compute_complementarity_metrics, parse_query, Budget,
    CanonicalUrl, DeduplicatedResult, DuplicateStatus, OriginalUrl, ProgressiveRoundTrace,
    Provider, ProviderAvailability, ProviderCapabilities, ProviderContext, ProviderError,
    ProviderExecutionStatus, ProviderItem, ProviderResult, Rank, RelevanceAssessmentStatus,
    RelevanceClassification, RelevanceThresholds, RoleAssignment, SearchOrchestrator,
    SCHEMA_VERSION,
};
use async_trait::async_trait;
use std::collections::BTreeMap;
use std::sync::Arc;

fn roles() -> RoleAssignment {
    RoleAssignment::primary_expansion("marginalia", ["searxng".to_string()])
}

fn deduped(
    url: &str,
    title: Option<&str>,
    snippet: Option<&str>,
    providers: &[&str],
) -> DeduplicatedResult {
    let parsed = url::Url::parse(url).unwrap();
    DeduplicatedResult {
        schema_version: SCHEMA_VERSION.into(),
        title: title.map(str::to_string),
        original_url: OriginalUrl(parsed.clone()),
        canonical_url: CanonicalUrl(parsed.clone()),
        original_urls: vec![OriginalUrl(parsed)],
        providers: providers.iter().map(|p| p.to_string()).collect(),
        representative_provider: providers.first().copied().unwrap_or("").into(),
        provider_ranks: providers
            .iter()
            .map(|p| ((*p).to_string(), Rank::new(1).ok()))
            .collect::<BTreeMap<_, _>>(),
        snippet: snippet.map(str::to_string),
        alternate_snippets: vec![],
        result_type: amatl_core::ResultType::Organic,
        published_at: None,
        author: None,
        language: None,
        file_type: None,
        thumbnail: None,
        metadata: BTreeMap::new(),
        observed_dates: vec![],
        duplicate_status: if providers.len() > 1 {
            DuplicateStatus::ConfirmedDuplicate
        } else {
            DuplicateStatus::Distinct
        },
        merge_reason: None,
        possible_duplicate_with: vec![],
    }
}

/// FIXTURE G — the critical test: a structurally unique expansion result that
/// is pure noise must add to UNIQUE_EXPANSION_RAW but not to
/// UNIQUE_RELEVANT_EXPANSION.
#[test]
fn unique_irrelevant_expansion_is_not_unique_relevant() {
    let query = parse_query("capital of Canada".into()).unwrap();
    let set = [
        deduped(
            "https://gov.example/ottawa",
            Some("The capital of Canada"),
            Some("Ottawa is the capital city of Canada."),
            &["marginalia"],
        ),
        deduped(
            "https://blog.example/opinions",
            Some("Political opinions and independent commentary"),
            Some("Personal essays about music, gardening and long walks."),
            &["searxng"],
        ),
    ];
    let comp = compute_complementarity_metrics(&set, &roles()).unwrap();
    assert_eq!(comp.unique_expansion, 1, "UNIQUE_EXPANSION_RAW preserved");

    let rel = assess_relevance(
        &query,
        &set,
        &roles(),
        &comp,
        &RelevanceThresholds::default(),
    )
    .unwrap();
    assert_eq!(rel.unique_relevant_expansion, Some(0));
    assert_eq!(rel.not_relevant_expansion, Some(1));
    assert_eq!(rel.raw_to_relevant_expansion_ratio, Some(0.0));
}

#[test]
fn unknown_results_do_not_count_as_confirmed_noise() {
    let query = parse_query("rust async programming".into()).unwrap();
    let set = [
        deduped("https://x.example/1", Some("Home"), None, &["searxng"]),
        deduped(
            "https://x.example/2",
            Some("Async programming in Rust"),
            Some("async and await in rust"),
            &["searxng"],
        ),
    ];
    let comp = compute_complementarity_metrics(&set, &roles()).unwrap();
    let rel = assess_relevance(
        &query,
        &set,
        &roles(),
        &comp,
        &RelevanceThresholds::default(),
    )
    .unwrap();
    assert_eq!(rel.unknown_expansion, Some(1));
    // one relevant, zero not-relevant conclusive -> 0.0, never NaN.
    let rate = rel.expansion_confirmed_noise_rate.unwrap();
    assert_eq!(rate, 0.0);
}

#[test]
fn identical_inputs_produce_identical_relevance() {
    let query = parse_query("rust async programming".into()).unwrap();
    let set = [deduped(
        "https://x.example/1",
        Some("Async programming in Rust"),
        Some("async await rust"),
        &["searxng"],
    )];
    let comp = compute_complementarity_metrics(&set, &roles()).unwrap();
    let a = assess_relevance(
        &query,
        &set,
        &roles(),
        &comp,
        &RelevanceThresholds::default(),
    );
    let b = assess_relevance(
        &query,
        &set,
        &roles(),
        &comp,
        &RelevanceThresholds::default(),
    );
    assert_eq!(a, b);
    let one = assess_result(&query, &set[0], &RelevanceThresholds::default());
    let two = assess_result(&query, &set[0], &RelevanceThresholds::default());
    assert_eq!(one, two);
}

#[test]
fn legacy_mode_returns_no_relevance() {
    let set = [deduped("https://x.example/1", Some("Rust"), None, &["p"])];
    // No complementarity in legacy mode, so no relevance either.
    assert!(compute_complementarity_metrics(&set, &RoleAssignment::legacy()).is_none());
}

// --- routing independence -------------------------------------------------

fn item(url: String, title: &str, snippet: Option<&str>) -> ProviderItem {
    ProviderItem {
        title: Some(title.into()),
        url,
        provider_rank: Some(Rank::new(1).unwrap()),
        snippet: snippet.map(str::to_string),
        result_type: None,
        published_at: None,
        author: None,
        language: None,
        file_type: None,
        thumbnail: None,
        metadata: BTreeMap::new(),
    }
}

struct ScriptedProvider {
    name: String,
    items: Vec<ProviderItem>,
}

#[async_trait]
impl Provider for ScriptedProvider {
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
        ProviderAvailability::Available
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
            results: self.items.clone(),
            accepted_filters: vec![],
            ignored_filters: vec![],
            approximated_filters: vec![],
            errors: vec![],
        })
    }
}

async fn run_trace(relevant_titles: bool) -> (Vec<ProgressiveRoundTrace>, Vec<String>) {
    let title = if relevant_titles {
        "rust async programming guide"
    } else {
        "totally unrelated gardening tips"
    };
    let marginalia = Arc::new(ScriptedProvider {
        name: "marginalia".into(),
        items: (0..3)
            .map(|i| item(format!("https://d{i}.m.example/p"), title, Some(title)))
            .collect(),
    });
    let searxng = Arc::new(ScriptedProvider {
        name: "searxng".into(),
        items: (0..3)
            .map(|i| item(format!("https://d{i}.s.example/p"), title, Some(title)))
            .collect(),
    });
    let providers: Vec<Arc<dyn Provider>> = vec![marginalia, searxng];
    let mut orchestrator =
        SearchOrchestrator::new(Budget::new(3, 5_000), 500).with_role_assignment(roles());
    let response = orchestrator
        .search(
            parse_query("rust async programming".into()).unwrap(),
            providers,
        )
        .await;
    let used = response.providers_used.clone();
    (orchestrator.routing_trace().to_vec(), used)
}

/// Changing the relevance assessment (via wildly different result text) must not
/// change the routing trace. Routing is structural only.
#[tokio::test]
async fn relevance_assessment_does_not_change_routing() {
    let (trace_relevant, used_relevant) = run_trace(true).await;
    let (trace_noise, used_noise) = run_trace(false).await;

    let selected = |t: &[ProgressiveRoundTrace]| {
        t.iter()
            .map(|r| (r.round, r.providers_selected.clone(), r.stop_reason.clone()))
            .collect::<Vec<_>>()
    };
    assert_eq!(selected(&trace_relevant), selected(&trace_noise));
    assert_eq!(used_relevant, used_noise);
}

/// The final response carries the relevance layer, and its absence/presence is
/// additive: legacy mode omits `complementarity` (and thus relevance) entirely.
#[tokio::test]
async fn search_response_serializes_relevance_additively() {
    let (_, _) = run_trace(true).await;
    let marginalia = Arc::new(ScriptedProvider {
        name: "marginalia".into(),
        items: (0..3)
            .map(|i| {
                item(
                    format!("https://d{i}.m.example/p"),
                    "rust async programming guide",
                    Some("async await in rust programming"),
                )
            })
            .collect(),
    });
    let searxng = Arc::new(ScriptedProvider {
        name: "searxng".into(),
        items: vec![item(
            "https://uniq.s.example/p".into(),
            "rust async programming deep dive",
            Some("a deep dive into async programming in rust"),
        )],
    });
    let providers: Vec<Arc<dyn Provider>> = vec![marginalia, searxng];
    let mut orchestrator =
        SearchOrchestrator::new(Budget::new(3, 5_000), 500).with_role_assignment(roles());
    let response = orchestrator
        .search(
            parse_query("rust async programming".into()).unwrap(),
            providers,
        )
        .await;

    let comp = response
        .complementarity
        .as_ref()
        .expect("roles active -> complementarity");
    assert_eq!(comp.relevance.status, RelevanceAssessmentStatus::Assessed);
    assert!(comp.relevance.relevant_primary.is_some());

    let json = serde_json::to_value(&response).unwrap();
    let rel = &json["complementarity"]["relevance"];
    assert_eq!(rel["status"], "assessed");
    // Additive: raw structural field still present and untouched.
    assert!(json["complementarity"]["unique_expansion"].is_number());
    // status unchanged by the new layer.
    assert!(json["status"].is_string());
}

#[test]
fn relevance_thresholds_are_marked_initial_heuristic_not_optimal() {
    // The defaults exist and are deterministic; this test documents that they
    // are the INITIAL_HEURISTIC and pins them so a change is deliberate.
    let t = RelevanceThresholds::default();
    assert_eq!(t.relevant_title_coverage, 0.75);
    assert_eq!(t.relevant_combined_coverage, 1.0);
    assert_eq!(t.possibly_relevant_coverage, 0.40);
    assert_eq!(t.not_relevant_ceiling, 0.25);
}

#[test]
fn fixture_table_matches_expected_classifications() {
    let cases: &[(&str, Option<&str>, Option<&str>, RelevanceClassification)] = &[
        (
            "rust async programming",
            Some("Async programming in Rust"),
            Some("Learn asynchronous programming using async and await in Rust."),
            RelevanceClassification::Relevant,
        ),
        (
            "capital of Canada",
            Some("Political opinions and independent commentary"),
            Some("Essays on music and gardening, with no mention of government."),
            RelevanceClassification::NotRelevant,
        ),
        (
            "rust async programming tokio",
            Some("Rust programming basics"),
            Some("An introduction to programming."),
            RelevanceClassification::PossiblyRelevant,
        ),
        (
            "rust async programming",
            Some("Rust async programming guide"),
            None,
            RelevanceClassification::Relevant,
        ),
        (
            "rust async programming",
            Some("Documentation index"),
            None,
            RelevanceClassification::Unknown,
        ),
        (
            "\"capital of Canada\"",
            Some("Geography facts"),
            Some("The capital of Canada is Ottawa, in Ontario."),
            RelevanceClassification::Relevant,
        ),
    ];
    for (q, title, snippet, expected) in cases {
        let query = parse_query((*q).to_string()).unwrap();
        let r = deduped("https://ex.example/p", *title, *snippet, &["p"]);
        let got = assess_result(&query, &r, &RelevanceThresholds::default()).classification;
        assert_eq!(got, *expected, "query={q:?} title={title:?}");
    }
}
