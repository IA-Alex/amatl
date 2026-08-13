use amatl_core::{
    parse_query, Budget, MockBehavior, MockProvider, Provider, ProviderErrorKind, ProviderItem,
    Rank, SearchOrchestrator, SearchStatus, SCHEMA_VERSION,
};
use std::sync::Arc;

fn item(provider_rank: u32, url: &str) -> ProviderItem {
    ProviderItem {
        title: Some("Rust async guide".into()),
        url: url.into(),
        provider_rank: Some(Rank::new(provider_rank).unwrap()),
        snippet: Some("A useful result".into()),
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
async fn valid_search_crosses_every_search_boundary() {
    let providers: Vec<Arc<dyn Provider>> = vec![
        Arc::new(MockProvider::success(
            "a",
            vec![item(1, "https://example.com/rust?utm_source=a")],
        )),
        Arc::new(MockProvider::success(
            "b",
            vec![item(2, "https://example.com/rust?utm_source=b")],
        )),
    ];
    let response = SearchOrchestrator::new(Budget::new(2, 8_000), 100)
        .search(parse_query("rust async".into()).unwrap(), providers)
        .await;
    assert_eq!(response.schema_version, SCHEMA_VERSION);
    assert_eq!(response.status, SearchStatus::Success);
    assert_eq!(response.results.len(), 1);
    assert_eq!(response.results[0].providers, ["a", "b"]);
    assert_eq!(response.results[0].canonical_url.0.query(), None);
}

#[tokio::test]
async fn partial_provider_result_is_normal_successful_output() {
    let providers: Vec<Arc<dyn Provider>> = vec![Arc::new(MockProvider::new(
        "partial",
        MockBehavior::Partial(
            vec![item(1, "https://example.com/rust")],
            ProviderErrorKind::RateLimit,
        ),
    ))];
    let response = SearchOrchestrator::new(Budget::new(1, 8_000), 100)
        .search(parse_query("rust".into()).unwrap(), providers)
        .await;
    assert_eq!(response.status, SearchStatus::PartialSuccess);
    assert_eq!(response.results.len(), 1);
    assert_eq!(response.providers_partial, ["partial"]);
}

#[tokio::test]
async fn all_provider_failures_are_global_failure() {
    let providers: Vec<Arc<dyn Provider>> = vec![Arc::new(MockProvider::new(
        "down",
        MockBehavior::Failure(ProviderErrorKind::Unavailable),
    ))];
    let response = SearchOrchestrator::new(Budget::new(1, 8_000), 100)
        .search(parse_query("rust".into()).unwrap(), providers)
        .await;
    assert_eq!(response.status, SearchStatus::Failure);
    assert!(response.results.is_empty());
    assert_eq!(response.providers_failed, ["down"]);
}

#[tokio::test]
async fn exhausted_budget_does_not_execute_unreserved_provider() {
    let providers: Vec<Arc<dyn Provider>> = vec![
        Arc::new(MockProvider::success(
            "a",
            vec![item(1, "https://a.example/rust")],
        )),
        Arc::new(MockProvider::success(
            "b",
            vec![item(1, "https://b.example/rust")],
        )),
    ];
    let response = SearchOrchestrator::new(Budget::new(1, 8_000), 100)
        .search(parse_query("rust".into()).unwrap(), providers)
        .await;
    assert_eq!(response.providers_used.len(), 1);
}
