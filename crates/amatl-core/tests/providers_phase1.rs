use amatl_core::{
    parse_query, Budget, HttpRequest, HttpResponse, HttpTransport, MarginaliaProvider,
    MockProvider, Provider, ProviderItem, Rank, SearchOrchestrator, SearchStatus,
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
/// response parsing through the global pipeline, and bounded retry. The retired
/// paid and self-hosted sources no longer ship an adapter to test.
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

/// H01 — the provider envelope is deduplicated across the full adapter →
/// pipeline path. Exact duplicate rows and tracking-parameter variants of one
/// URL collapse to a single visible result, so the "two identical rows"
/// symptom can only come from *distinct* canonical URLs (scheme / `www`
/// aliases), never from a repeated URL or a duplicated pipeline stage.
#[tokio::test]
async fn h01_exact_duplicate_rows_collapse_across_the_pipeline() {
    let transport = Arc::new(FixtureTransport::new(
        200,
        BTreeMap::new(),
        br#"{"results":[
          {"title":"Search engine components and architecture","url":"https://www.example.org/","description":"same"},
          {"title":"Search engine components and architecture","url":"https://www.example.org/","description":"same"},
          {"title":"Search engine components and architecture","url":"https://www.example.org/?utm_source=x","description":"same"}
        ]}"#,
    ));
    let providers: Vec<Arc<dyn Provider>> = vec![Arc::new(MarginaliaProvider::new(
        Some("fixture-token".to_owned().into()),
        true,
        true,
        transport.clone(),
    ))];
    let response = SearchOrchestrator::new(Budget::new(1, 8_000), 100)
        .search(
            parse_query("open source search engine architecture".into()).unwrap(),
            providers,
        )
        .await;
    assert_eq!(response.results.len(), 1);
    assert_eq!(
        response.results[0].canonical_url.0.as_str(),
        "https://www.example.org/"
    );
    assert_eq!(response.results[0].rank.get(), 1);
}

/// H02 — AMATL never fabricates or re-assembles snippet text. The provider
/// `description` reaches the public result verbatim after HTML-entity decoding
/// and whitespace collapsing only: separators such as `//`, abrupt context
/// changes or concatenated prose are the provider's own text, so AMATL
/// documents the provider limit instead of rewriting meaning.
#[tokio::test]
async fn h02_snippet_is_provider_text_passed_through_unchanged() {
    let provider_snippet = "Historia de la computacion // Mexico / Pioneros y maquinas";
    let body = format!(
        r#"{{"results":[{{"title":"Historia","url":"https://hist.example/mx","description":"{provider_snippet}"}}]}}"#
    );
    let transport = Arc::new(FixtureTransport::new(200, BTreeMap::new(), body.as_bytes()));
    let providers: Vec<Arc<dyn Provider>> = vec![Arc::new(MarginaliaProvider::new(
        Some("fixture-token".to_owned().into()),
        true,
        true,
        transport.clone(),
    ))];
    let response = SearchOrchestrator::new(Budget::new(1, 8_000), 100)
        .search(
            parse_query("historia de la computacion en Mexico".into()).unwrap(),
            providers,
        )
        .await;
    assert_eq!(
        response.results[0].snippet.as_deref(),
        Some(provider_snippet),
        "the snippet must be the provider text, unmodified"
    );
}

/// H03 — with a single source, equal title coverage must not reorder the
/// provider's native ranking. Marginalia's rank order is carried as
/// `provider_rank`; the reranker only moves results whose query evidence
/// actually differs, so a same-evidence set keeps the provider order.
#[tokio::test]
async fn h03_native_order_survives_when_evidence_is_equal() {
    let transport = Arc::new(FixtureTransport::new(
        200,
        BTreeMap::new(),
        br#"{"results":[
          {"title":"alpha beta gamma","url":"https://one.example/a","description":"alpha beta gamma"},
          {"title":"alpha beta gamma","url":"https://two.example/b","description":"alpha beta gamma"},
          {"title":"alpha beta gamma","url":"https://three.example/c","description":"alpha beta gamma"}
        ]}"#,
    ));
    let providers: Vec<Arc<dyn Provider>> = vec![Arc::new(MarginaliaProvider::new(
        Some("fixture-token".to_owned().into()),
        true,
        true,
        transport.clone(),
    ))];
    let response = SearchOrchestrator::new(Budget::new(1, 8_000), 100)
        .search(parse_query("alpha beta gamma".into()).unwrap(), providers)
        .await;
    let urls: Vec<_> = response
        .results
        .iter()
        .map(|result| result.canonical_url.0.as_str())
        .collect();
    assert_eq!(
        urls,
        vec![
            "https://one.example/a",
            "https://two.example/b",
            "https://three.example/c"
        ],
        "equal evidence must preserve the provider's native order"
    );
}
