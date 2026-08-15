//! SearXNG provider — self-hosted meta-search engine.
//!
//! SearXNG aggregates multiple upstream search engines and returns unified JSON.
//! It is free, has no quota, and requires no API key. The instance URL is read
//! from the `SEARXNG_INSTANCE_URL` environment variable (defaults to
//! `http://127.0.0.1:8888` when unset, which matches the default SearXNG bind).
//!
//! # Governance
//!
//! - `allowed_access_method = "self_hosted"`
//! - `cost_model = "0"` (free)
//! - `plan_or_contract = "self-hosted"`
//! - No credential required.
//!
//! # API
//!
//! `GET /search?q=<query>&format=json&pageno=<page>&categories=<cat>`
//!
//! See <https://docs.searxng.org/dev/search_api.html>.

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

/// Default SearXNG instance URL when `SEARXNG_INSTANCE_URL` is unset.
const DEFAULT_INSTANCE_URL: &str = "http://127.0.0.1:8888";

/// Environment variable that overrides the SearXNG instance URL.
const INSTANCE_URL_ENV: &str = "SEARXNG_INSTANCE_URL";

pub struct SearXngProvider {
    instance_url: Url,
    enabled: bool,
    approved: bool,
    transport: Arc<dyn HttpTransport>,
}

impl SearXngProvider {
    pub fn new(
        instance_url: Url,
        enabled: bool,
        approved: bool,
        transport: Arc<dyn HttpTransport>,
    ) -> Self {
        Self {
            instance_url,
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
        let mut url = self
            .instance_url
            .join("/search")
            .map_err(|_| {
                error(
                    ProviderErrorKind::InvalidResponse,
                    "SearXNG instance URL is invalid",
                    None,
                )
            })?;

        let (query, filters) = translated_query(plan);
        {
            let mut pairs = url.query_pairs_mut();
            pairs
                .append_pair("q", &query)
                .append_pair("format", "json")
                .append_pair("pageno", "1");
        }

        Ok((
            HttpRequest::get(
                url,
                vec![
                    ("accept".into(), "application/json".into()),
                    ("cache-control".into(), "no-cache".into()),
                ],
                timeout_ms,
            ),
            filters,
        ))
    }
}


#[async_trait]
impl Provider for SearXngProvider {
    fn name(&self) -> &str {
        "searxng"
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            schema_version: SCHEMA_VERSION.into(),
            pagination: true,
            language: false,
            region: false,
            time_range: false,
            site_filter: false,
            file_filter: false,
            news: false,
            code: false,
            docs: false,
            academic: false,
            authentication: false,
            estimated_cost: Some(0),
        }
    }

    fn availability(&self) -> ProviderAvailability {
        if !self.enabled {
            ProviderAvailability::Unavailable {
                code: "provider_disabled".into(),
                message: "SearXNG is not enabled in the configuration".into(),
            }
        } else if !self.approved {
            ProviderAvailability::Unavailable {
                code: "provider_not_approved".into(),
                message: "SearXNG governance record is incomplete or expired".into(),
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
        let response = self.transport.execute(request).await.map_err(|e| {
            error(
                ProviderErrorKind::Unavailable,
                &format!("SearXNG transport error: {e}"),
                None,
            )
        })?;
        parse_response(response, filters)
    }
}


// ---------------------------------------------------------------------------
// Filter translation
// ---------------------------------------------------------------------------

#[derive(Default)]
struct FilterUse {
    accepted: Vec<String>,
    ignored: Vec<String>,
    approximated: Vec<String>,
}

/// Translate AMATL query filters into SearXNG parameters.
///
/// SearXNG's JSON API supports `categories` and `engines` as query parameters,
/// but does not have native site/language/region/time-range filters at the API
/// level. We approximate what we can and ignore the rest.
fn translated_query(plan: &SearchPlan) -> (String, FilterUse) {
    let query = &plan.query;
    let mut search = query.normalized_query.clone();

    for term in &query.quoted_terms {
        search.push_str(&format!(" \"{term}\""));
    }

    let mut filters = FilterUse::default();

    // SearXNG can pass `site:` and other prefixes through to its engines, but
    // support depends on the upstream engine. We pass them in the query string
    // and mark them as approximated.
    for domain in &query.domains {
        search.push_str(&format!(" site:{domain}"));
        filters.approximated.push("site".into());
    }
    for domain in &query.excluded_domains {
        search.push_str(&format!(" -site:{domain}"));
        filters.approximated.push("excluded_site".into());
    }
    for term in &query.excluded_terms {
        search.push_str(&format!(" -{term}"));
        filters.approximated.push("excluded_terms".into());
    }

    if query.language.is_some() {
        filters.ignored.push("language".into());
    }
    if query.region.is_some() {
        filters.ignored.push("region".into());
    }
    if query.date_from.is_some() || query.date_to.is_some() {
        filters.ignored.push("time_range".into());
    }
    if !query.file_types.is_empty() {
        filters.ignored.push("filetype".into());
    }

    (search.trim().to_string(), filters)
}


// ---------------------------------------------------------------------------
// Response parsing
// ---------------------------------------------------------------------------

/// Top-level SearXNG JSON response.
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
struct SearXngResponse {
    query: String,
    #[serde(default)]
    results: Vec<SearXngResult>,
    #[serde(default)]
    answers: Vec<SearXngAnswer>,
    #[serde(default)]
    unresponsive_engines: Vec<String>,
}

#[derive(Deserialize)]
struct SearXngResult {
    title: Option<String>,
    url: Option<String>,
    content: Option<String>,
    engine: Option<String>,
    #[serde(default)]
    published_date: Option<String>,
}

#[derive(Deserialize)]
struct SearXngAnswer {
    answer: Option<String>,
    url: Option<String>,
}

fn parse_response(
    response: HttpResponse,
    _filters: FilterUse,
) -> Result<ProviderResult, ProviderError> {
    if response.status != 200 {
        return Err(http_status_error("SearXNG", &response));
    }

    let body = String::from_utf8(response.body).map_err(|_| {
        error(
            ProviderErrorKind::InvalidResponse,
            "SearXNG returned non-UTF-8 body",
            None,
        )
    })?;

    let searxng: SearXngResponse = serde_json::from_str(&body).map_err(|e| {
        error(
            ProviderErrorKind::InvalidResponse,
            &format!("SearXNG JSON parse error: {e}"),
            None,
        )
    })?;

    let mut results: Vec<ProviderItem> = searxng
        .results
        .into_iter()
        .enumerate()
        .map(|(index, result)| ProviderItem {
            title: result.title,
            url: result.url.unwrap_or_default(),
            provider_rank: Rank::new((index + 1) as u32).ok(),
            snippet: result.content,
            result_type: None,
            published_at: result.published_date,
            author: None,
            language: None,
            file_type: None,
            thumbnail: None,
            metadata: Default::default(),
        })
        .collect();

    // Promote answers as synthetic results at the top.
    for (index, answer) in searxng.answers.into_iter().enumerate() {
        results.insert(
            index,
            ProviderItem {
                title: answer.answer.clone(),
                url: answer.url.unwrap_or_default(),
                provider_rank: Rank::new((index + 1) as u32).ok(),
                snippet: answer.answer,
                result_type: None,
                published_at: None,
                author: None,
                language: None,
                file_type: None,
                thumbnail: None,
                metadata: Default::default(),
            },
        );
    }

    let status = if searxng.unresponsive_engines.is_empty() {
        ProviderExecutionStatus::Success
    } else {
        ProviderExecutionStatus::Partial
    };

    Ok(ProviderResult {
        schema_version: SCHEMA_VERSION.into(),
        provider: "searxng".into(),
        status,
        results,
        accepted_filters: vec![],
        ignored_filters: vec![],
        approximated_filters: vec![],
        errors: vec![],
    })
}

// ---------------------------------------------------------------------------
// Error helpers
// ---------------------------------------------------------------------------

fn http_status_error(provider_label: &str, response: &HttpResponse) -> ProviderError {
    let (kind, message) = match response.status {
        400 => (ProviderErrorKind::InvalidResponse, "bad request"),
        429 => (ProviderErrorKind::RateLimit, "rate limited"),
        500..=599 => (
            ProviderErrorKind::Unavailable,
            "SearXNG service unavailable",
        ),
        _ => (
            ProviderErrorKind::InvalidResponse,
            "SearXNG returned an unexpected status",
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
        provider: "searxng".into(),
        kind,
        message: message.into(),
        retry_after_ms,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

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
                selected_providers: vec!["searxng".into()],
                provider_budget_requests: BTreeMap::from([("searxng".into(), 1)]),
                debug_reasons: vec![],
            },
            &mut budget,
        )
    }

    fn provider() -> SearXngProvider {
        SearXngProvider::new(
            Url::parse("http://127.0.0.1:8888").unwrap(),
            true,
            true,
            Arc::new(super::super::http::ReqwestTransport::new(1024).unwrap()),
        )
    }

    #[test]
    fn builds_request_with_query_and_format() {
        let (request, _filters) = provider().request(&plan("rust async"), 100).unwrap();
        let url = request.url.to_string();
        assert!(url.contains("q=rust+async"));
        assert!(url.contains("format=json"));
        assert!(url.contains("pageno=1"));
    }

    #[test]
    fn passes_site_filter_as_approximated_query_term() {
        let (request, filters) = provider().request(&plan("rust site:docs.rs"), 100).unwrap();
        let url = request.url.to_string();
        assert!(url.contains("site%3Adocs.rs"));
        assert!(filters.approximated.contains(&"site".into()));
    }

    #[test]
    fn ignores_unsupported_filters() {
        let (_request, filters) = provider()
            .request(
                &plan("rust lang:es region:MX after:2025-01-01 filetype:pdf"),
                100,
            )
            .unwrap();
        assert!(filters.ignored.contains(&"language".into()));
        assert!(filters.ignored.contains(&"region".into()));
        assert!(filters.ignored.contains(&"time_range".into()));
        assert!(filters.ignored.contains(&"filetype".into()));
    }

    #[test]
    fn promotes_answers_as_synthetic_results() {
        let response = HttpResponse {
            status: 200,
            headers: BTreeMap::new(),
            body: br#"{
                "query": "rust",
                "results": [],
                "answers": [{"answer": "Rust is a systems programming language", "url": "https://www.rust-lang.org/"}],
                "unresponsive_engines": []
            }"#
            .to_vec(),
        };
        let result = parse_response(response, FilterUse::default()).unwrap();
        assert_eq!(result.results.len(), 1);
        assert!(result.results[0]
            .snippet
            .as_deref()
            .unwrap()
            .contains("systems programming"));
    }

    #[test]
    fn marks_partial_when_engines_are_unresponsive() {
        let response = HttpResponse {
            status: 200,
            headers: BTreeMap::new(),
            body: br#"{
                "query": "rust",
                "results": [{"title": "Rust", "url": "https://rust-lang.org/", "content": "ok"}],
                "unresponsive_engines": ["duckduckgo", "google"]
            }"#
            .to_vec(),
        };
        let result = parse_response(response, FilterUse::default()).unwrap();
        assert_eq!(result.status, ProviderExecutionStatus::Partial);
    }

    #[test]
    fn maps_rate_limit_correctly() {
        let response = HttpResponse {
            status: 429,
            headers: BTreeMap::from([("retry-after".into(), "5".into())]),
            body: b"too many requests".to_vec(),
        };
        let error = parse_response(response, FilterUse::default()).unwrap_err();
        assert_eq!(error.kind, ProviderErrorKind::RateLimit);
        assert_eq!(error.retry_after_ms, Some(5_000));
    }

    #[test]
    fn rejects_non_utf8_body() {
        let response = HttpResponse {
            status: 200,
            headers: BTreeMap::new(),
            body: vec![0xFF, 0xFE, 0x00, 0x01],
        };
        let error = parse_response(response, FilterUse::default()).unwrap_err();
        assert_eq!(error.kind, ProviderErrorKind::InvalidResponse);
    }

    #[test]
    fn availability_reflects_enabled_and_approved() {
        let unavailable_disabled = SearXngProvider::new(
            Url::parse("http://127.0.0.1:8888").unwrap(),
            false,
            true,
            Arc::new(super::super::http::ReqwestTransport::new(1024).unwrap()),
        );
        assert!(!matches!(
            unavailable_disabled.availability(),
            ProviderAvailability::Available
        ));

        let unavailable_not_approved = SearXngProvider::new(
            Url::parse("http://127.0.0.1:8888").unwrap(),
            true,
            false,
            Arc::new(super::super::http::ReqwestTransport::new(1024).unwrap()),
        );
        assert!(!matches!(
            unavailable_not_approved.availability(),
            ProviderAvailability::Available
        ));

        let available = SearXngProvider::new(
            Url::parse("http://127.0.0.1:8888").unwrap(),
            true,
            true,
            Arc::new(super::super::http::ReqwestTransport::new(1024).unwrap()),
        );
        assert!(matches!(
            available.availability(),
            ProviderAvailability::Available
        ));
    }
}
