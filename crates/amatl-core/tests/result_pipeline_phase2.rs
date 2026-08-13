use amatl_core::{
    parse_query, Budget, MockProvider, Provider, ProviderItem, Rank, ResultStatus,
    SearchOrchestrator, SearchStatus,
};
use std::collections::BTreeMap;
use std::sync::Arc;

fn item(url: &str, rank: u32, title: Option<&str>) -> ProviderItem {
    ProviderItem {
        title: title.map(str::to_string),
        url: url.into(),
        provider_rank: Some(Rank::new(rank).unwrap()),
        snippet: Some("useful snippet".into()),
        result_type: None,
        published_at: None,
        author: None,
        language: None,
        file_type: None,
        thumbnail: None,
        metadata: BTreeMap::new(),
    }
}

#[tokio::test]
async fn degraded_fields_are_public_and_do_not_discard_the_item() {
    let mut value = item("https://example.com/", 1, None);
    value.snippet = Some("broken\u{fffd}snippet".into());
    value.published_at = Some("2026-02-30".into());
    let providers: Vec<Arc<dyn Provider>> = vec![Arc::new(MockProvider::success("p", vec![value]))];
    let response = SearchOrchestrator::new(Budget::new(1, 1_000), 100)
        .search(parse_query("rust".into()).unwrap(), providers)
        .await;
    assert_eq!(response.status, SearchStatus::PartialSuccess);
    assert_eq!(response.results.len(), 1);
    assert!(response.results[0].snippet.is_none());
    assert!(response
        .degradations
        .iter()
        .any(|item| item.code == "invalid_snippet"));
    assert!(response
        .degradations
        .iter()
        .any(|item| item.code == "invalid_published_at"));
}

#[tokio::test]
async fn confirmed_duplicate_keeps_all_contributing_providers() {
    let providers: Vec<Arc<dyn Provider>> = vec![
        Arc::new(MockProvider::success(
            "a",
            vec![item(
                "https://example.com/rust?utm_source=a",
                1,
                Some("Rust guide"),
            )],
        )),
        Arc::new(MockProvider::success(
            "b",
            vec![item(
                "https://example.com/rust?utm_source=b",
                2,
                Some("Rust guide"),
            )],
        )),
    ];
    let response = SearchOrchestrator::new(Budget::new(2, 1_000), 100)
        .search(parse_query("rust".into()).unwrap(), providers)
        .await;
    assert_eq!(response.status, SearchStatus::Success);
    assert_eq!(response.results.len(), 1);
    assert_eq!(response.results[0].providers, ["a", "b"]);
    assert_eq!(
        response.results[0].canonical_url.0.as_str(),
        "https://example.com/rust"
    );
}

#[tokio::test]
async fn provider_diversity_limit_relegates_without_deleting() {
    let values = (1..=6)
        .map(|rank| {
            item(
                &format!("https://domain{rank}.example/result"),
                rank,
                Some("result"),
            )
        })
        .collect();
    let providers: Vec<Arc<dyn Provider>> = vec![Arc::new(MockProvider::success("p", values))];
    let response = SearchOrchestrator::new(Budget::new(1, 1_000), 100)
        .search(parse_query("result".into()).unwrap(), providers)
        .await;
    assert_eq!(response.results.len(), 6);
    assert_eq!(
        response.results[5].status,
        ResultStatus::RelegatedByDiversity
    );
    assert_eq!(response.results[5].rank, Rank::new(6).unwrap());
}

#[tokio::test]
async fn all_invalid_items_produce_typed_failure_not_empty_partial_success() {
    let providers: Vec<Arc<dyn Provider>> = vec![Arc::new(MockProvider::success(
        "p",
        vec![item("javascript:alert(1)", 1, Some("invalid"))],
    ))];
    let response = SearchOrchestrator::new(Budget::new(1, 1_000), 100)
        .search(parse_query("result".into()).unwrap(), providers)
        .await;
    assert_eq!(response.status, SearchStatus::Failure);
    assert!(response.results.is_empty());
    assert!(response
        .errors
        .iter()
        .any(|error| error.code == "no_usable_results"));
}
