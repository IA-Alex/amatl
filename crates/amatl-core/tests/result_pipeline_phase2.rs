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
async fn naive_published_date_is_preserved_and_does_not_degrade() {
    // A search engine can report `publishedDate` as a *naive* local timestamp
    // (`YYYY-MM-DDTHH:MM:SS`, no offset). `normalize_date` only accepted RFC 3339
    // (offset required) or a bare `YYYY-MM-DD`, so every such date was dropped
    // and turned a clean response into a `partial_success` via
    // `invalid_published_at`. First observed against a live instance
    // (2026-11-14): ~11 of 35 results per query.
    let mut value = item("https://example.com/", 1, Some("Rust"));
    value.published_at = Some("2024-06-11T00:00:00".into());
    let providers: Vec<Arc<dyn Provider>> = vec![Arc::new(MockProvider::success("p", vec![value]))];
    let response = SearchOrchestrator::new(Budget::new(1, 8_000), 100)
        .search(parse_query("rust".into()).unwrap(), providers)
        .await;

    assert_eq!(response.status, SearchStatus::Success);
    assert_eq!(response.results.len(), 1);
    assert_eq!(
        response.results[0].published_at.as_deref(),
        Some("2024-06-11T00:00:00Z"),
        "a naive timestamp must be preserved as UTC, not discarded"
    );
    assert!(!response
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
    // 8 000 ms keeps the deadline far above `minimum_remaining_deadline_ms`
    // (750 ms): a `DeadlineNear` stop would degrade this `Success` status.
    let response = SearchOrchestrator::new(Budget::new(2, 8_000), 100)
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

/// H04 — an ambiguous single term keeps every legitimate interpretation. AMATL
/// performs no disambiguation, entity resolution or synonym rewriting: each
/// distinct document the provider returns for `mercury` (planet, element,
/// language) survives as its own visible result, so the user can still explore
/// the senses. Ranking only reorders; it never deletes a sense.
#[tokio::test]
async fn h04_ambiguous_term_keeps_every_interpretation() {
    let values = vec![
        item(
            "https://solar.example/mercury-planet",
            1,
            Some("Mercury (planet)"),
        ),
        item(
            "https://chem.example/mercury-element",
            2,
            Some("Mercury (element)"),
        ),
        item(
            "https://lang.example/mercury-lang",
            3,
            Some("Mercury programming language"),
        ),
    ];
    let providers: Vec<Arc<dyn Provider>> = vec![Arc::new(MockProvider::success("p", values))];
    let response = SearchOrchestrator::new(Budget::new(1, 1_000), 100)
        .search(parse_query("mercury".into()).unwrap(), providers)
        .await;
    let mut urls: Vec<_> = response
        .results
        .iter()
        .map(|result| result.canonical_url.0.as_str())
        .collect();
    urls.sort_unstable();
    assert_eq!(
        urls,
        vec![
            "https://chem.example/mercury-element",
            "https://lang.example/mercury-lang",
            "https://solar.example/mercury-planet",
        ],
        "every sense must survive; none may be collapsed away"
    );
    assert!(response
        .results
        .iter()
        .all(|result| result.status == ResultStatus::Visible));
}
