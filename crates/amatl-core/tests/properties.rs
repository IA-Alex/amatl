use amatl_core::{
    canonical::canonicalize, dedupe::deduplicate, normalize::normalize, CanonicalResult,
    CanonicalUrl, CanonicalizationStatus, FieldProvenance, NormalizedResult, OriginalUrl,
    ProviderExecutionStatus, ProviderItem, ProviderResult, ResultType, SCHEMA_VERSION,
};
use proptest::prelude::*;
use std::collections::BTreeMap;

fn normalized(url: url::Url) -> NormalizedResult {
    NormalizedResult {
        schema_version: SCHEMA_VERSION.into(),
        title: Some("stable property title with enough tokens".into()),
        raw_url: url.as_str().into(),
        url: OriginalUrl(url),
        provider: "property".into(),
        provider_rank: None,
        snippet: None,
        result_type: ResultType::Organic,
        published_at: None,
        author: None,
        language: None,
        file_type: None,
        thumbnail: None,
        metadata: BTreeMap::new(),
        provenance: BTreeMap::from([("url".into(), FieldProvenance::Reported)]),
        degradations: vec![],
    }
}

fn canonical(provider: &str, url: url::Url) -> CanonicalResult {
    let mut result = canonicalize(normalized(url));
    result.provider = provider.into();
    result
}

proptest! {
    #[test]
    fn query_parser_preserves_raw_input(raw in ".{1,256}") {
        if let Ok(query) = amatl_core::parse_query(raw.clone()) {
            prop_assert!(query.secondary_contract_lists_are_not_applicable());
            prop_assert_eq!(&query.raw_query, &raw);
        }
    }

    #[test]
    fn accepted_search_urls_are_http_without_credentials(
        host in "[a-z]{1,12}\\.[a-z]{2,6}",
        path in "[a-z0-9/_-]{0,48}"
    ) {
        let raw = format!("https://{host}/{path}");
        let provider = ProviderResult {
            schema_version: SCHEMA_VERSION.into(),
            provider: "property".into(),
            status: ProviderExecutionStatus::Success,
            results: vec![ProviderItem {
                title: None,
                url: raw,
                provider_rank: None,
                snippet: None,
                result_type: None,
                published_at: None,
                author: None,
                language: None,
                file_type: None,
                thumbnail: None,
                metadata: BTreeMap::new(),
            }],
            accepted_filters: vec![],
            ignored_filters: vec![],
            approximated_filters: vec![],
            errors: vec![],
        };
        let (values, _) = normalize(&[provider]);
        prop_assert_eq!(values.len(), 1);
        let url = &values[0].url.0;
        prop_assert!(matches!(url.scheme(), "http" | "https"));
        prop_assert!(url.username().is_empty());
        prop_assert!(url.password().is_none());
    }

    #[test]
    fn canonicalization_is_idempotent_for_safe_urls(
        host in "[a-z]{1,12}\\.[a-z]{2,6}",
        path in "[a-z0-9/_-]{0,48}",
        id in 0_u32..100_000
    ) {
        let url = url::Url::parse(&format!("https://{host}/{path}?utm_source=test&id={id}")).unwrap();
        let first = canonicalize(normalized(url));
        let second = canonicalize(normalized(first.canonical_url.0.clone()));
        prop_assert_eq!(first.canonical_url, second.canonical_url);
        prop_assert_eq!(second.canonicalization_status, CanonicalizationStatus::Complete);
        prop_assert!(second.transformations.is_empty());
    }

    #[test]
    fn dedupe_merges_exact_canonical_urls_once(
        copies in 1_usize..12,
        id in 0_u32..100_000
    ) {
        let url = url::Url::parse(&format!("https://example.com/resource/{id}")).unwrap();
        let input = (0..copies)
            .map(|index| canonical(&format!("provider-{index}"), url.clone()))
            .collect();
        let output = deduplicate(input);
        prop_assert_eq!(output.len(), 1);
        prop_assert_eq!(output[0].providers.len(), copies);
        prop_assert_eq!(&output[0].canonical_url, &CanonicalUrl(url));
    }
}

trait QueryPropertyExtension {
    fn secondary_contract_lists_are_not_applicable(&self) -> bool;
}

impl QueryPropertyExtension for amatl_core::Query {
    fn secondary_contract_lists_are_not_applicable(&self) -> bool {
        self.domains.iter().all(|value| !value.is_empty())
            && self.file_types.iter().all(|value| !value.is_empty())
    }
}

#[test]
fn value_types_reject_invalid_serialized_values() {
    assert!(serde_json::from_str::<amatl_core::Rank>("0").is_err());
    assert!(serde_json::from_str::<amatl_core::Rank>("1").is_ok());
    assert!(serde_json::from_str::<amatl_core::RankingScore>("-0.1").is_err());
    assert!(serde_json::from_str::<amatl_core::RankingScore>("1.1").is_err());
    assert!(serde_json::from_str::<amatl_core::RankingScore>("0.5").is_ok());
}
