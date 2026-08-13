use amatl_core::{
    parse_query, BraveProvider, Budget, HttpRequest, HttpResponse, HttpTransport, MockProvider,
    Provider, ProviderItem, Rank, SearchOrchestrator, SearchStatus,
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

#[tokio::test]
async fn brave_without_token_is_omitted_without_network_access() {
    let transport = Arc::new(FixtureTransport::new(200, BTreeMap::new(), b"{}"));
    let providers: Vec<Arc<dyn Provider>> = vec![
        Arc::new(BraveProvider::new(None, true, true, transport.clone())),
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
async fn brave_fixture_crosses_provider_and_global_pipeline() {
    let transport = Arc::new(FixtureTransport::new(
        200,
        BTreeMap::new(),
        br#"{"web":{"results":[{"title":"Rust","url":"https://example.com/?utm_source=brave","description":"Guide"}]}}"#,
    ));
    let providers: Vec<Arc<dyn Provider>> = vec![Arc::new(BraveProvider::new(
        Some("fixture-token".into()),
        true,
        true,
        transport.clone(),
    ))];
    let response = SearchOrchestrator::new(Budget::new(1, 1_000), 100)
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
    let providers: Vec<Arc<dyn Provider>> = vec![Arc::new(BraveProvider::new(
        Some("fixture-token".into()),
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
