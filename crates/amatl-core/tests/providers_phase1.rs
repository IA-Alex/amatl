use amatl_core::{
    parse_query, Budget, HttpRequest, HttpResponse, HttpTransport, MarginaliaProvider,
    MockProvider, Provider, ProviderItem, Rank, SearXngProvider, SearchOrchestrator, SearchStatus,
};
use async_trait::async_trait;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

struct FixtureTransport {
    response: HttpResponse,
    calls: AtomicUsize,
}

impl FixtureTransport {
    fn new(status: u16, headers: BTreeMap<String, String>, body: &[u8]) -> Self {
        Self {
            response: HttpResponse {
                status,
                headers,
                body: body.to_vec(),
            },
            calls: AtomicUsize::new(0),
        }
    }
}

#[async_trait]
impl HttpTransport for FixtureTransport {
    async fn execute(&self, _request: HttpRequest) -> Result<HttpResponse, String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(self.response.clone())
    }
}

fn item() -> ProviderItem {
    ProviderItem {
        title: Some("Local result".into()),
        url: "https://local.example/".into(),
        provider_rank: Some(Rank::new(1).unwrap()),
        snippet: None,
        result_type: None,
        published_at: None,
        author: None,
        language: None,
        file_type: None,
        thumbnail: None,
        metadata: Default::default(),
    }
}

/// The one shipped source that needs a credential is Marginalia, so it carries
/// the end-to-end coverage for the whole key-gated path: credential handling,
/// response parsing through the global pipeline, and bounded retry. SearXNG is
/// credential-free and covered by the characterization suite; the retired paid
/// sources no longer ship an adapter to test.
#[tokio::test]
async fn key_gated_provider_without_credential_is_omitted_without_network_access() {
    let transport = Arc::new(FixtureTransport::new(200, BTreeMap::new(), b"{}"));
    let providers: Vec<Arc<dyn Provider>> = vec![
        Arc::new(MarginaliaProvider::new(None, true, true, transport.clone())),
        Arc::new(MockProvider::success("local", vec![item()])),
    ];
    let response = SearchOrchestrator::new(Budget::new(2, 1_000), 100)
        .search(parse_query("rust".into()).unwrap(), providers)
        .await;
    assert_eq!(transport.calls.load(Ordering::SeqCst), 0);
    assert_eq!(response.status, SearchStatus::PartialSuccess);
    assert!(response
        .degradations
        .iter()
        .any(|item| item.code == "provider_credential_missing"));
}

#[tokio::test]
async fn marginalia_fixture_crosses_provider_and_global_pipeline() {
    let transport = Arc::new(FixtureTransport::new(
        200,
        BTreeMap::new(),
        br#"{"results":[{"title":"Rust","url":"https://example.com/?utm_source=marginalia","description":"Guide"}]}"#,
    ));
    let providers: Vec<Arc<dyn Provider>> = vec![Arc::new(MarginaliaProvider::new(
        Some("fixture-token".to_owned().into()),
        true,
        true,
        transport.clone(),
    ))];
    // 8 000 ms keeps the deadline far above `minimum_remaining_deadline_ms`
    // (750 ms): a `DeadlineNear` stop would degrade this `Success` status.
    let response = SearchOrchestrator::new(Budget::new(1, 8_000), 100)
        .search(parse_query("rust".into()).unwrap(), providers)
        .await;
    assert_eq!(response.status, SearchStatus::Success);
    assert_eq!(
        response.results[0].canonical_url.0.as_str(),
        "https://example.com/"
    );
    assert_eq!(transport.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn real_adapter_rate_limit_uses_bounded_retry_policy() {
    let transport = Arc::new(FixtureTransport::new(429, BTreeMap::new(), b"private body"));
    let providers: Vec<Arc<dyn Provider>> = vec![Arc::new(MarginaliaProvider::new(
        Some("fixture-token".to_owned().into()),
        true,
        true,
        transport.clone(),
    ))];
    let response = SearchOrchestrator::new(Budget::new(1, 1_000), 100)
        .search(parse_query("rust".into()).unwrap(), providers)
        .await;
    assert_eq!(transport.calls.load(Ordering::SeqCst), 2);
    assert_eq!(response.status, SearchStatus::Failure);
    assert!(response
        .errors
        .iter()
        .any(|error| error.code == "provider_rate_limit"));
    assert!(!response
        .errors
        .iter()
        .any(|error| error.message.contains("private")));
    assert!(response.errors.iter().all(|error| error.recoverable));
}

// ---------------------------------------------------------------------------
// SearXNG end-to-end coverage through the real orchestrator.
//
// The suite above exercises Marginalia for the whole pipeline because it is the
// key-gated source. SearXNG — the credential-free primary shipped source — was
// only ever covered by isolated `request()`/`parse_response()` unit tests, so
// nothing proved that a real SearXNG JSON body survives routing, the parallel
// execution layer, canonicalization, dedupe and ranking. These tests close
// that gap with a deterministic fixture transport (no network).

fn searxng(transport: Arc<FixtureTransport>, enabled: bool, approved: bool) -> Arc<dyn Provider> {
    Arc::new(SearXngProvider::new(
        url::Url::parse("http://127.0.0.1:8888").unwrap(),
        enabled,
        approved,
        transport,
    ))
}

#[tokio::test]
async fn searxng_fixture_crosses_provider_and_global_pipeline() {
    // A real-instance-shaped body, including the extra per-result fields and
    // the `[engine, reason]` pair shape of `unresponsive_engines`.
    let transport = Arc::new(FixtureTransport::new(
        200,
        BTreeMap::new(),
        br#"{
            "query": "rust async",
            "results": [
                {"title":"Async Rust","url":"https://docs.rs/tokio/","content":"Tokio guide","engines":["google"],"positions":[1]},
                {"title":"Rust Lang","url":"https://www.rust-lang.org/?utm_source=x","content":"Home"},
                {"title":"Duplicate","url":"https://docs.rs/tokio/","content":"same url"}
            ],
            "answers": [],
            "unresponsive_engines": []
        }"#,
    ));
    let providers: Vec<Arc<dyn Provider>> = vec![searxng(transport.clone(), true, true)];
    // 8 000 ms keeps the deadline above `minimum_remaining_deadline_ms` so a
    // `DeadlineNear` stop does not degrade an otherwise clean run.
    let response = SearchOrchestrator::new(Budget::new(1, 8_000), 100)
        .search(parse_query("rust async".into()).unwrap(), providers)
        .await;

    assert_eq!(transport.calls.load(Ordering::SeqCst), 1);
    assert_eq!(response.status, SearchStatus::Success);
    assert_eq!(response.providers_used, vec!["searxng".to_owned()]);
    assert!(response.providers_failed.is_empty());
    assert!(response.providers_partial.is_empty());
    // The third entry repeats a canonical URL, so it is deduplicated away.
    assert_eq!(response.results.len(), 2, "{:?}", response.results);
    // Canonicalization strips tracking parameters (`?utm_source=x`).
    assert_eq!(
        response.results[1].canonical_url.0.as_str(),
        "https://www.rust-lang.org/"
    );
    assert!(response
        .results
        .iter()
        .all(|result| result.providers.contains(&"searxng".to_owned())));
}

#[tokio::test]
async fn searxng_unresponsive_engines_surface_as_partial_through_the_pipeline() {
    let transport = Arc::new(FixtureTransport::new(
        200,
        BTreeMap::new(),
        br#"{
            "query": "rust",
            "results": [{"title":"Rust","url":"https://rust-lang.org/","content":"ok"}],
            "answers": [],
            "unresponsive_engines": [["startpage", "Suspended: CAPTCHA"]]
        }"#,
    ));
    let providers: Vec<Arc<dyn Provider>> = vec![searxng(transport, true, true)];
    let response = SearchOrchestrator::new(Budget::new(1, 8_000), 100)
        .search(parse_query("rust".into()).unwrap(), providers)
        .await;

    assert_eq!(response.status, SearchStatus::PartialSuccess);
    assert_eq!(response.providers_used, vec!["searxng".to_owned()]);
    assert_eq!(response.providers_partial, vec!["searxng".to_owned()]);
    assert_eq!(response.results.len(), 1);
}

#[tokio::test]
async fn searxng_empty_result_set_is_a_typed_failure_not_a_panic() {
    // The characterization campaign observed a real instance answering HTTP
    // 200 with `results: []`. That must resolve to a typed failure/degradation
    // at the aggregated layer, never a panic or a bogus success.
    let transport = Arc::new(FixtureTransport::new(
        200,
        BTreeMap::new(),
        br#"{"query":"rust","results":[],"answers":[],"unresponsive_engines":[]}"#,
    ));
    let providers: Vec<Arc<dyn Provider>> = vec![searxng(transport, true, true)];
    let response = SearchOrchestrator::new(Budget::new(1, 8_000), 100)
        .search(parse_query("rust".into()).unwrap(), providers)
        .await;

    assert_eq!(response.status, SearchStatus::Failure);
    assert!(response.results.is_empty());
    assert_eq!(response.providers_used, vec!["searxng".to_owned()]);
}

#[tokio::test]
async fn searxng_filters_it_cannot_honor_are_routed_out_instead_of_silently_ignored() {
    // Documented routing contract (characterization campaign): a filter that
    // requires a capability SearXNG does not declare must exclude it *before*
    // any network access, emitting `no_available_provider`. `site:` needs
    // `site_filter` and `lang:` needs `language`; SearXNG declares neither, so
    // no upstream request may be made rather than silently dropping the filter.
    for query in ["rust site:docs.rs", "rust lang:es"] {
        let transport = Arc::new(FixtureTransport::new(
            200,
            BTreeMap::new(),
            br#"{"query":"rust","results":[],"answers":[],"unresponsive_engines":[]}"#,
        ));
        let providers: Vec<Arc<dyn Provider>> = vec![searxng(transport.clone(), true, true)];
        let response = SearchOrchestrator::new(Budget::new(1, 8_000), 100)
            .search(parse_query(query.into()).unwrap(), providers)
            .await;

        assert_eq!(
            transport.calls.load(Ordering::SeqCst),
            0,
            "{query}: SearXNG must not be called for a filter it cannot honor"
        );
        assert!(
            response
                .errors
                .iter()
                .any(|error| error.code == "no_available_provider"),
            "{query}: expected no_available_provider, got {:?}",
            response.errors
        );
    }
}

#[tokio::test]
async fn searxng_disabled_and_unapproved_are_omitted_without_network_access() {
    let transport = Arc::new(FixtureTransport::new(
        200,
        BTreeMap::new(),
        br#"{"query":"rust","results":[{"title":"Rust","url":"https://rust-lang.org/"}]}"#,
    ));

    for (enabled, approved, expected_code) in [
        (false, true, "provider_disabled"),
        (true, false, "provider_not_approved"),
    ] {
        let providers: Vec<Arc<dyn Provider>> = vec![searxng(transport.clone(), enabled, approved)];
        let response = SearchOrchestrator::new(Budget::new(1, 8_000), 100)
            .search(parse_query("rust".into()).unwrap(), providers)
            .await;

        assert_eq!(transport.calls.load(Ordering::SeqCst), 0, "{expected_code}");
        assert!(
            response
                .degradations
                .iter()
                .any(|degradation| degradation.code == expected_code),
            "expected {expected_code}, got {:?}",
            response.degradations
        );
        assert!(response.results.is_empty(), "{expected_code}");
    }
}

#[tokio::test]
async fn searxng_http_error_statuses_map_to_typed_provider_errors() {
    // 503 must become a recoverable `provider_unavailable`, and 429 a
    // `provider_rate_limit` — both with bounded retry (max_retries = 1, so two
    // transport attempts). The retry must fit inside the global deadline; a
    // short `retry-after` keeps the backoff well under a generous budget.
    for (status, expected_code) in [(503, "provider_unavailable"), (429, "provider_rate_limit")] {
        let transport = Arc::new(FixtureTransport::new(
            status,
            BTreeMap::from([("retry-after".to_owned(), "1".to_owned())]),
            b"upstream busy",
        ));
        let providers: Vec<Arc<dyn Provider>> = vec![searxng(transport.clone(), true, true)];
        let response = SearchOrchestrator::new(Budget::new(1, 8_000), 100)
            .search(parse_query("rust".into()).unwrap(), providers)
            .await;

        assert_eq!(response.status, SearchStatus::Failure, "{status}");
        assert!(response.results.is_empty(), "{status}");
        assert_eq!(
            response.providers_failed,
            vec!["searxng".to_owned()],
            "{status}"
        );
        assert!(
            response
                .errors
                .iter()
                .any(|error| error.code == expected_code),
            "{status}: expected {expected_code}, got {:?}",
            response.errors
        );
        assert!(
            response.errors.iter().all(|error| error.recoverable),
            "{status}: {:?}",
            response.errors
        );
        // max_retries = 1 → exactly two transport attempts.
        assert_eq!(transport.calls.load(Ordering::SeqCst), 2, "{status}");
    }
}

#[tokio::test]
async fn searxng_drops_results_without_a_url_like_marginalia() {
    // Adapter-contract parity: Marginalia filters out items whose `url` is
    // empty before they ever reach the pipeline. SearXNG used to emit
    // `url: ""` for any result with a missing/null URL, forcing the shared
    // `normalize` stage to reject each one and record a spurious per-item
    // degradation while inflating the provider's reported result count. A
    // result the caller cannot open must not be reported as a result at all.
    let transport = Arc::new(FixtureTransport::new(
        200,
        BTreeMap::new(),
        br#"{
            "query": "rust",
            "results": [
                {"title":"Has URL","url":"https://rust-lang.org/","content":"ok"},
                {"title":"No URL field","content":"missing url"},
                {"title":"Null URL","url":null,"content":"null url"},
                {"title":"Empty URL","url":"","content":"empty url"}
            ],
            "answers": [],
            "unresponsive_engines": []
        }"#,
    ));
    let providers: Vec<Arc<dyn Provider>> = vec![searxng(transport, true, true)];
    let response = SearchOrchestrator::new(Budget::new(1, 8_000), 100)
        .search(parse_query("rust".into()).unwrap(), providers)
        .await;

    assert_eq!(response.status, SearchStatus::Success);
    assert_eq!(response.results.len(), 1);
    assert_eq!(
        response.results[0].canonical_url.0.as_str(),
        "https://rust-lang.org/"
    );
    // The three malformed entries must be discarded by the adapter itself,
    // never surfacing as pipeline degradations.
    assert!(
        response
            .degradations
            .iter()
            .all(|degradation| !degradation.code.contains("url")),
        "adapter must drop URL-less results, not leave them for normalize: {:?}",
        response.degradations
    );
}
