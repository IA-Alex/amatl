use amatl_core::{
    analyze_evidence_v2, canonical::canonicalize, dedupe::deduplicate, normalize::normalize,
    CanonicalResult, CanonicalUrl, CanonicalizationStatus, Document, DocumentStatus, FetchMethod,
    FieldProvenance, FinalUrl, NormalizedResult, OriginalUrl, ProviderExecutionStatus,
    ProviderItem, ProviderResult, ResultType, EVIDENCE_V2_FRAGMENT_BYTES,
    EVIDENCE_V2_MAX_FRAGMENTS, SCHEMA_VERSION,
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

fn evidence_document(content: String) -> Document {
    let url = url::Url::parse("https://example.com/property").unwrap();
    Document {
        schema_version: SCHEMA_VERSION.into(),
        search_result_id: "property-document".into(),
        original_url: OriginalUrl(url.clone()),
        canonical_url: CanonicalUrl(url.clone()),
        final_url: FinalUrl(url),
        content_hash: "source-content-hash".into(),
        fetch_method: FetchMethod::Http,
        extractor_used: Some("property-v1".into()),
        content_type: Some("text/plain".into()),
        size: content.len() as u64,
        retrieved_at: "2026-08-13T00:00:00Z".into(),
        status: DocumentStatus::Enriched,
        content: Some(content),
        title: None,
        author: None,
        published_at: None,
        metadata: BTreeMap::new(),
    }
}

/// Sufijos de host que la política SSRF bloquea deliberadamente en
/// `crates/amatl-core/src/security.rs` (`blocked_hostname`); el catálogo
/// completo está documentado en `docs/security/ssrf-controls.md`
/// (sección "Block catalog"). Los tests de este archivo verifican
/// propiedades sobre URLs que `normalize()` **acepta**; `normalize()` descarta
/// los ítems que fallan `validate_search_url`, así que un host generado como
/// `a.lan` sería rechazado a propósito y no constituye un fallo del código.
const SSRF_BLOCKED_HOST_SUFFIXES: [&str; 7] = [
    ".localhost",
    ".local",
    ".localdomain",
    ".internal",
    ".intranet",
    ".lan",
    ".home",
];

/// Host de dos etiquetas con sufijo que la política SSRF no bloquea: mantiene
/// la forma del generador original (`[a-z]{1,12}\.[a-z]{2,6}`) pero excluye
/// los TLD reservados a redes locales (`.lan`, `.local`, `.home`, ...) para
/// que la propiedad "toda URL bien formada con host público produce 1
/// resultado aceptado" siga siendo válida y significativa.
fn public_host() -> impl Strategy<Value = String> {
    "[a-z]{1,12}\\.[a-z]{2,6}".prop_filter("host must not end in an SSRF-blocked suffix", |host| {
        !SSRF_BLOCKED_HOST_SUFFIXES
            .iter()
            .any(|suffix| host.ends_with(suffix))
    })
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
        host in public_host(),
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
        host in public_host(),
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

    #[test]
    fn evidence_v2_ranges_are_exact_and_utf8_safe(content in ".{0,2048}") {
        let query = amatl_core::parse_query("evidence property".into()).unwrap();
        let documents = vec![evidence_document(content.clone())];
        let first = analyze_evidence_v2(&query, &documents);
        prop_assert_eq!(&first, &analyze_evidence_v2(&query, &documents));
        prop_assert_eq!(first.len(), 1);
        prop_assert!(first[0].fragments.len() <= EVIDENCE_V2_MAX_FRAGMENTS);
        for fragment in &first[0].fragments {
            let start = usize::try_from(fragment.start_byte).unwrap();
            let end = usize::try_from(fragment.end_byte).unwrap();
            prop_assert!(start < end);
            prop_assert!(content.is_char_boundary(start));
            prop_assert!(content.is_char_boundary(end));
            prop_assert_eq!(&fragment.text, &content[start..end]);
            prop_assert!(fragment.text.len() <= EVIDENCE_V2_FRAGMENT_BYTES);
            prop_assert_eq!(fragment.fragment_hash.len(), 64);
            prop_assert_eq!(
                &fragment.provenance_id,
                &first[0].provenance.provenance_id
            );
        }
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
