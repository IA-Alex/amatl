//! Marginalia Search API provider.
//!
//! Marginalia is an independent search engine focused on non-commercial content
//! and text-heavy websites. It offers a public API at
//! <https://api2.marginalia-search.com/>.
//!
//! `api.marginalia.nu` (the original endpoint) is deprecated; `api2.*` is the
//! current one, verified in `docs/gobernanza-providers.md`.
//!
//! # API
//!
//! ```text
//! GET https://api2.marginalia-search.com/search?query=<query>&count=<n>
//! API-Key: <api_key>
//! ```
//!
//! Response shape:
//! ```json
//! {
//!   "results": [
//!     { "title": "...", "url": "...", "description": "...", "domain": "..." }
//!   ]
//! }
//! ```

use super::{
    HttpRequest, HttpResponse, HttpTransport, Provider, ProviderAvailability, ProviderContext,
};
use crate::model::{
    ProviderCapabilities, ProviderError, ProviderErrorKind, ProviderExecutionStatus, ProviderItem,
    ProviderResult, Rank, SearchPlan, SCHEMA_VERSION,
};
use async_trait::async_trait;
use serde::Deserialize;
use std::sync::Arc;
use url::Url;

const ENDPOINT: &str = "https://api2.marginalia-search.com/search";

pub struct MarginaliaProvider {
    api_key: Option<String>,
    enabled: bool,
    approved: bool,
    transport: Arc<dyn HttpTransport>,
}

impl MarginaliaProvider {
    pub fn new(
        api_key: Option<String>,
        enabled: bool,
        approved: bool,
        transport: Arc<dyn HttpTransport>,
    ) -> Self {
        Self {
            api_key,
            enabled,
            approved,
            transport,
        }
    }

    fn request(
        &self,
        plan: &SearchPlan,
        timeout_ms: u64,
    ) -> Result<(HttpRequest, FilterUse), ProviderError> {
        let key = self.api_key.as_ref().ok_or_else(|| {
            error(
                ProviderErrorKind::Auth,
                "Marginalia credential is unavailable",
                None,
            )
        })?;
        let mut url = Url::parse(ENDPOINT).map_err(|_| {
            error(
                ProviderErrorKind::InvalidResponse,
                "Marginalia endpoint configuration is invalid",
                None,
            )
        })?;
        let (query, filters) = translated_query(plan);
        {
            let mut pairs = url.query_pairs_mut();
            pairs
                .append_pair("query", &query)
                .append_pair("count", "20");
        }
        Ok((
            HttpRequest::get(
                url,
                vec![
                    ("accept".into(), "application/json".into()),
                    ("cache-control".into(), "no-cache".into()),
                    ("api-key".into(), key.clone()),
                ],
                timeout_ms,
            ),
            filters,
        ))
    }
}

#[async_trait]
impl Provider for MarginaliaProvider {
    fn name(&self) -> &str {
        "marginalia"
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            schema_version: SCHEMA_VERSION.into(),
            pagination: true,
            language: false,
            region: false,
            time_range: false,
            site_filter: true,
            file_filter: false,
            news: false,
            code: false,
            docs: false,
            academic: false,
            authentication: true,
            estimated_cost: Some(0), // Free — no paid tier
        }
    }

    fn availability(&self) -> ProviderAvailability {
        if !self.enabled {
            ProviderAvailability::Unavailable {
                code: "provider_disabled".into(),
                message: "Marginalia is not enabled in the configuration".into(),
            }
        } else if !self.approved {
            ProviderAvailability::Unavailable {
                code: "provider_not_approved".into(),
                message: "Marginalia governance record is incomplete or expired".into(),
            }
        } else if self.api_key.is_none() {
            ProviderAvailability::Unavailable {
                code: "credential_missing".into(),
                message: "MARGINALIA_API_KEY is not set".into(),
            }
        } else {
            ProviderAvailability::Available
        }
    }

    async fn search(
        &self,
        plan: &SearchPlan,
        context: &ProviderContext,
    ) -> Result<ProviderResult, ProviderError> {
        let (request, filters) = self.request(plan, context.timeout_ms)?;
        let response = self.transport.execute(request).await.map_err(|_| {
            error(
                ProviderErrorKind::Network,
                "Marginalia network request failed",
                None,
            )
        })?;
        parse_response(response, filters)
    }
}

// ── Filter translation ──────────────────────────────────────────────────────

#[derive(Debug, Default)]
struct FilterUse {
    accepted: Vec<String>,
    ignored: Vec<String>,
    approximated: Vec<String>,
}

/// Translates the structured query into Marginalia's flat query syntax.
/// Marginalia supports `site:` natively; other filters are ignored.
fn translated_query(plan: &SearchPlan) -> (String, FilterUse) {
    let query = &plan.query;
    let mut terms = vec![query.normalized_query.clone()];
    terms.extend(query.quoted_terms.iter().map(|term| format!("\"{term}\"")));
    terms.extend(query.excluded_terms.iter().map(|term| format!("-{term}")));
    terms.extend(query.domains.iter().map(|domain| format!("site:{domain}")));
    terms.extend(
        query
            .excluded_domains
            .iter()
            .map(|domain| format!("-site:{domain}")),
    );
    let mut accepted = vec![];
    let mut ignored = vec![];
    if !query.domains.is_empty() || !query.excluded_domains.is_empty() {
        accepted.push("site".into());
    }
    if !query.file_types.is_empty() {
        ignored.push("filetype".into());
    }
    if query.language.is_some() {
        ignored.push("language".into());
    }
    if query.region.is_some() {
        ignored.push("region".into());
    }
    if query.date_from.is_some() || query.date_to.is_some() {
        ignored.push("time_range".into());
    }
    (
        terms
            .into_iter()
            .filter(|term| !term.is_empty())
            .collect::<Vec<_>>()
            .join(" "),
        FilterUse {
            accepted,
            ignored,
            approximated: vec![],
        },
    )
}

// ── Response parsing ─────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct MarginaliaResponse {
    #[serde(default)]
    results: Vec<MarginaliaResult>,
}

#[derive(Deserialize)]
struct MarginaliaResult {
    title: Option<String>,
    url: String,
    description: Option<String>,
    #[serde(default)]
    domain: Option<String>,
}

fn parse_response(
    response: HttpResponse,
    filters: FilterUse,
) -> Result<ProviderResult, ProviderError> {
    if response.status != 200 {
        return Err(status_error("Marginalia", &response));
    }
    let envelope: MarginaliaResponse = serde_json::from_slice(&response.body).map_err(|_| {
        error(
            ProviderErrorKind::InvalidResponse,
            "Marginalia returned invalid JSON",
            None,
        )
    })?;
    let results = envelope
        .results
        .into_iter()
        .enumerate()
        .map(|(index, item)| ProviderItem {
            title: item.title,
            url: item.url,
            provider_rank: Rank::new(index as u32 + 1).ok(),
            snippet: item.description,
            result_type: None,
            published_at: None,
            author: None,
            language: None,
            file_type: None,
            thumbnail: None,
            metadata: {
                let mut meta = std::collections::BTreeMap::new();
                if let Some(domain) = item.domain {
                    meta.insert("domain".into(), domain);
                }
                meta
            },
        })
        .collect();
    Ok(ProviderResult {
        schema_version: SCHEMA_VERSION.into(),
        provider: "marginalia".into(),
        status: ProviderExecutionStatus::Success,
        results,
        accepted_filters: filters.accepted,
        ignored_filters: filters.ignored,
        approximated_filters: filters.approximated,
        errors: vec![],
    })
}

// ── Error helpers ────────────────────────────────────────────────────────────

fn status_error(provider_label: &str, response: &HttpResponse) -> ProviderError {
    let (kind, message) = match response.status {
        401 | 403 => (ProviderErrorKind::Auth, "provider rejected credentials"),
        429 => (ProviderErrorKind::RateLimit, "provider rate limit exceeded"),
        400..=499 => (
            ProviderErrorKind::InvalidResponse,
            "provider rejected the request",
        ),
        500..=599 => (
            ProviderErrorKind::Unavailable,
            "provider service unavailable",
        ),
        _ => (
            ProviderErrorKind::InvalidResponse,
            "provider returned an unexpected status",
        ),
    };
    let retry_after_ms = response
        .headers
        .get("retry-after")
        .and_then(|value| value.parse::<u64>().ok())
        .map(|seconds| seconds.saturating_mul(1000));
    error(
        kind,
        &format!("{provider_label}: {message}"),
        retry_after_ms,
    )
}

fn error(kind: ProviderErrorKind, message: &str, retry_after_ms: Option<u64>) -> ProviderError {
    ProviderError {
        schema_version: SCHEMA_VERSION.into(),
        provider: "marginalia".into(),
        kind,
        message: message.into(),
        retry_after_ms,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::planning::build_search_plan;
    use crate::router::RoutingRecommendation;
    use crate::{classify, parse_query, Budget};
    use std::collections::BTreeMap;

    fn plan(raw: &str) -> SearchPlan {
        let query = parse_query(raw.into()).unwrap();
        let mut budget = Budget::new(1, 1_000);
        build_search_plan(
            query.clone(),
            classify(&query),
            RoutingRecommendation {
                selected_providers: vec!["marginalia".into()],
                provider_budget_requests: BTreeMap::from([("marginalia".into(), 1)]),
                debug_reasons: vec![],
            },
            &mut budget,
        )
    }

    // ── Availability contracts ───────────────────────────────────────────

    #[test]
    fn missing_credential_degrades_to_credential_missing() {
        let provider = MarginaliaProvider::new(
            None,
            true,
            true,
            Arc::new(super::super::http::ReqwestTransport::new(1024).unwrap()),
        );
        match provider.availability() {
            ProviderAvailability::Unavailable { code, .. } => {
                assert_eq!(code, "credential_missing");
            }
            other => panic!("expected credential_missing, got {other:?}"),
        }
    }

    #[test]
    fn with_key_enabled_and_approved_reports_available() {
        let provider = MarginaliaProvider::new(
            Some("test-key".into()),
            true,
            true,
            Arc::new(super::super::http::ReqwestTransport::new(1024).unwrap()),
        );
        assert!(matches!(
            provider.availability(),
            ProviderAvailability::Available
        ));
    }

    #[test]
    fn disabled_provider_reports_provider_disabled() {
        let provider = MarginaliaProvider::new(
            Some("test-key".into()),
            false,
            true,
            Arc::new(super::super::http::ReqwestTransport::new(1024).unwrap()),
        );
        match provider.availability() {
            ProviderAvailability::Unavailable { code, .. } => {
                assert_eq!(code, "provider_disabled");
            }
            other => panic!("expected provider_disabled, got {other:?}"),
        }
    }

    #[test]
    fn not_approved_reports_provider_not_approved() {
        let provider = MarginaliaProvider::new(
            Some("test-key".into()),
            true,
            false,
            Arc::new(super::super::http::ReqwestTransport::new(1024).unwrap()),
        );
        match provider.availability() {
            ProviderAvailability::Unavailable { code, .. } => {
                assert_eq!(code, "provider_not_approved");
            }
            other => panic!("expected provider_not_approved, got {other:?}"),
        }
    }

    // ── Filter translation contracts ─────────────────────────────────────

    #[test]
    fn maps_site_filter_and_ignores_unsupported() {
        let provider = MarginaliaProvider::new(
            Some("test-key".into()),
            true,
            true,
            Arc::new(super::super::http::ReqwestTransport::new(1024).unwrap()),
        );
        let (request, filters) = provider
            .request(
                &plan("rust site:docs.rs lang:es region:MX filetype:pdf"),
                100,
            )
            .unwrap();
        let url = request.url.to_string();
        assert!(url.contains("site%3Adocs.rs"));
        assert!(filters.accepted.contains(&"site".into()));
        assert!(filters.ignored.contains(&"language".into()));
        assert!(filters.ignored.contains(&"region".into()));
        assert!(filters.ignored.contains(&"filetype".into()));
        assert!(!url.contains("test-key"));
    }

    /// Pins the verified API contract (`docs/gobernanza-providers.md`):
    /// `api2.marginalia-search.com`, the `query` parameter and the `API-Key`
    /// header. `api.marginalia.nu` is deprecated and must not regress in.
    #[test]
    fn uses_the_current_api2_endpoint_query_param_and_header() {
        let provider = MarginaliaProvider::new(
            Some("test-key".into()),
            true,
            true,
            Arc::new(super::super::http::ReqwestTransport::new(1024).unwrap()),
        );
        let (request, _) = provider.request(&plan("rust"), 100).unwrap();
        assert_eq!(request.url.host_str(), Some("api2.marginalia-search.com"));
        assert!(request
            .url
            .query_pairs()
            .any(|(key, value)| key == "query" && value == "rust"));
        assert!(request
            .headers
            .iter()
            .any(|(name, value)| name == "api-key" && value == "test-key"));
        assert!(!request
            .headers
            .iter()
            .any(|(name, _)| name == "authorization"));
    }

    #[test]
    fn credential_is_not_leaked_in_url() {
        let provider = MarginaliaProvider::new(
            Some("top-secret".into()),
            true,
            true,
            Arc::new(super::super::http::ReqwestTransport::new(1024).unwrap()),
        );
        let (request, _) = provider.request(&plan("rust"), 100).unwrap();
        let sanitized = request.sanitized_url().to_string();
        assert!(!sanitized.contains("top-secret"));
    }

    #[test]
    fn missing_credential_fails_with_auth() {
        let provider = MarginaliaProvider::new(
            None,
            true,
            true,
            Arc::new(super::super::http::ReqwestTransport::new(1024).unwrap()),
        );
        match provider.request(&plan("rust"), 100) {
            Err(error) => assert_eq!(error.kind, ProviderErrorKind::Auth),
            Ok(_) => panic!("expected auth error"),
        }
    }

    // ── Response parsing contracts ───────────────────────────────────────

    #[test]
    fn parses_fixture_and_preserves_native_order() {
        let response = HttpResponse {
            status: 200,
            headers: BTreeMap::new(),
            body: br#"{"results":[{"title":"One","url":"https://one.example/","description":"First"},{"title":"Two","url":"https://two.example/"}]}"#.to_vec(),
        };
        let result = parse_response(response, FilterUse::default()).unwrap();
        assert_eq!(result.results.len(), 2);
        assert_eq!(result.results[0].provider_rank, Rank::new(1).ok());
        assert_eq!(result.results[1].provider_rank, Rank::new(2).ok());
        assert_eq!(result.results[0].title.as_deref(), Some("One"));
    }

    #[test]
    fn stores_domain_in_metadata_when_present() {
        let response = HttpResponse {
            status: 200,
            headers: BTreeMap::new(),
            body: br#"{"results":[{"title":"Rust","url":"https://rust-lang.org/","description":"Rust lang","domain":"rust-lang.org"}]}"#.to_vec(),
        };
        let result = parse_response(response, FilterUse::default()).unwrap();
        assert_eq!(
            result.results[0].metadata.get("domain").map(|s| s.as_str()),
            Some("rust-lang.org")
        );
    }

    #[test]
    fn empty_results_is_valid() {
        let response = HttpResponse {
            status: 200,
            headers: BTreeMap::new(),
            body: br#"{"results":[]}"#.to_vec(),
        };
        let result = parse_response(response, FilterUse::default()).unwrap();
        assert!(result.results.is_empty());
        assert_eq!(result.status, ProviderExecutionStatus::Success);
    }

    #[test]
    fn maps_rate_limit_and_retry_after_safely() {
        let response = HttpResponse {
            status: 429,
            headers: BTreeMap::from([("retry-after".into(), "2".into())]),
            body: b"secret upstream body".to_vec(),
        };
        let error = parse_response(response, FilterUse::default()).unwrap_err();
        assert_eq!(error.kind, ProviderErrorKind::RateLimit);
        assert_eq!(error.retry_after_ms, Some(2_000));
        assert!(!error.message.contains("secret"));
    }

    #[test]
    fn auth_failure_is_typed_correctly() {
        let response = HttpResponse {
            status: 401,
            headers: BTreeMap::new(),
            body: b"unauthorized".to_vec(),
        };
        let error = parse_response(response, FilterUse::default()).unwrap_err();
        assert_eq!(error.kind, ProviderErrorKind::Auth);
    }

    #[test]
    fn invalid_json_is_parser_error() {
        let response = HttpResponse {
            status: 200,
            headers: BTreeMap::new(),
            body: b"not json".to_vec(),
        };
        let error = parse_response(response, FilterUse::default()).unwrap_err();
        assert_eq!(error.kind, ProviderErrorKind::InvalidResponse);
    }

    #[test]
    fn server_error_is_unavailable() {
        let response = HttpResponse {
            status: 503,
            headers: BTreeMap::new(),
            body: b"down".to_vec(),
        };
        let error = parse_response(response, FilterUse::default()).unwrap_err();
        assert_eq!(error.kind, ProviderErrorKind::Unavailable);
    }

    proptest::proptest! {
        /// No response body, however malformed, should ever panic the parser —
        /// only a typed `ProviderError` is an acceptable outcome. Mirrors the
        /// arbitrary-bytes pattern already used for the local-ingest parsers
        /// in `ingest.rs`.
        #[test]
        fn parser_never_panics_on_arbitrary_bytes(
            status in proptest::num::u16::ANY,
            body in proptest::collection::vec(proptest::num::u8::ANY, 0..4096)
        ) {
            let response = HttpResponse {
                status,
                headers: BTreeMap::new(),
                body,
            };
            let _ = parse_response(response, FilterUse::default());
        }
    }
}
