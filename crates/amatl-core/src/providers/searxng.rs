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
        let mut url = self.instance_url.join("/search").map_err(|_| {
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
        let response = self.transport.execute(request).await.map_err(|_| {
            // Parity with the other adapters: a transport failure is
            // `Network`, not `Unavailable`, and the message is fixed. The
            // transport layer (`http.rs`) already logs the classified cause
            // against a sanitized URL; reqwest's own `Display` can embed the
            // request URL (query string included), so it must not reach the
            // serialized `ProviderError` message.
            error(
                ProviderErrorKind::Network,
                "SearXNG network request failed",
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
    #[serde(default)]
    results: Vec<SearXngResult>,
    #[serde(default)]
    answers: Vec<SearXngAnswer>,
    /// `[engine_name, error_type]` pairs, e.g. `["example_engine", "too many
    /// requests"]`. A plain `Vec<String>` here previously produced "invalid
    /// type: sequence, expected a string" on any real SearXNG instance,
    /// since every element is itself a 2-item array, not a string — this
    /// only ever passed against the fixtures below because they were typed
    /// (incorrectly) to match the old field, not a real response.
    #[serde(default)]
    unresponsive_engines: Vec<(String, String)>,
}

#[derive(Deserialize)]
struct SearXngResult {
    title: Option<String>,
    url: Option<String>,
    content: Option<String>,
    #[serde(default, rename = "publishedDate", alias = "published_date")]
    published_date: Option<String>,
}

#[derive(Deserialize)]
struct SearXngAnswer {
    answer: Option<String>,
    url: Option<String>,
}

fn parse_response(
    response: HttpResponse,
    filters: FilterUse,
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
        // A result with no URL is not a result the caller can open. Drop it
        // here, exactly as the Marginalia adapter does, so it never reaches
        // the shared pipeline — where it would be rejected item by item,
        // recording a spurious degradation and turning a clean response into
        // `partial_success`.
        .filter_map(|result| {
            let url = result.url?;
            if url.is_empty() {
                return None;
            }
            Some((result.title, url, result.content, result.published_date))
        })
        .enumerate()
        .map(
            |(index, (title, url, content, published_date))| ProviderItem {
                title,
                url,
                provider_rank: Rank::new((index + 1) as u32).ok(),
                snippet: content,
                result_type: None,
                published_at: published_date,
                author: None,
                language: None,
                file_type: None,
                thumbnail: None,
                metadata: Default::default(),
            },
        )
        .collect();

    // Promote answers as synthetic results at the top.
    //
    // An answer with no URL cannot be opened by the caller, so it is dropped
    // for the same reason a URL-less result is: inserting it with `url: ""`
    // hands the shared pipeline an item it rejects item by item, recording an
    // `invalid_url` degradation and turning an otherwise clean response into
    // `partial_success`. Answers that carry a usable URL are still promoted.
    for (offset, answer) in searxng
        .answers
        .into_iter()
        .filter_map(|answer| {
            let url = answer.url.filter(|value| !value.is_empty())?;
            Some((url, answer.answer))
        })
        .enumerate()
    {
        results.insert(
            offset,
            ProviderItem {
                title: answer.1.clone(),
                url: answer.0,
                provider_rank: None,
                snippet: answer.1,
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
    // Renumber over the final vector so ranks stay a unique ordinal (answers
    // and results previously both started at 1).
    for (index, item) in results.iter_mut().enumerate() {
        item.provider_rank = Rank::new((index + 1) as u32).ok();
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
        // SearXNG's JSON API has no native filter support (see the module
        // doc comment), so `FilterUse` never carries an `accepted` list for
        // this provider — everything is either approximated in the query
        // string or ignored outright.
        accepted_filters: vec![],
        ignored_filters: filters.ignored,
        approximated_filters: filters.approximated,
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

    /// Captures `tracing` events emitted while running `f`, so a warning that is
    /// the only observable difference can be asserted instead of assumed.
    fn capture_warnings(f: impl FnOnce()) -> Vec<String> {
        use std::sync::{Arc, Mutex};
        struct Capture(Arc<Mutex<Vec<String>>>);
        impl tracing::Subscriber for Capture {
            fn enabled(&self, _: &tracing::Metadata<'_>) -> bool {
                true
            }
            fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
                tracing::span::Id::from_u64(1)
            }
            fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
            fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
            fn event(&self, event: &tracing::Event<'_>) {
                struct Visit<'a>(&'a mut String);
                impl tracing::field::Visit for Visit<'_> {
                    fn record_debug(
                        &mut self,
                        field: &tracing::field::Field,
                        value: &dyn std::fmt::Debug,
                    ) {
                        if field.name() == "message" {
                            self.0.push_str(&format!("{value:?}"));
                        }
                    }
                }
                let mut message = String::new();
                event.record(&mut Visit(&mut message));
                self.0.lock().unwrap().push(message);
            }
            fn enter(&self, _: &tracing::span::Id) {}
            fn exit(&self, _: &tracing::span::Id) {}
        }
        let buffer = Arc::new(Mutex::new(Vec::new()));
        let subscriber = Capture(buffer.clone());
        tracing::subscriber::with_default(subscriber, f);
        let captured = buffer.lock().unwrap();
        captured.clone()
    }

    #[test]
    fn factory_warns_only_when_the_instance_url_is_present_but_unparseable() {
        use crate::providers::registry::resolve_searxng_instance_url;

        // `SearXngFactory::build` is the only seam that turns `credential_env`
        // (a URL, not a secret) into a provider, and `build` cannot return an
        // error. Before this, `.ok()` discarded a malformed value exactly like
        // a missing one, so `SEARXNG_INSTANCE_URL=not-a-url` queried a
        // different instance and surfaced only as `provider_network` — read by
        // an operator as "the server is down". The fallback stays; the warning
        // is what makes the two states distinguishable, so it is the thing
        // under test.
        let warned = capture_warnings(|| {
            resolve_searxng_instance_url("searxng", Some("not-a-url"));
        });
        assert_eq!(warned.len(), 1, "a malformed URL must warn: {warned:?}");
        assert!(warned[0].contains("not a valid URL"), "{warned:?}");

        // A scheme-less `host:port` is not a URL either and used to become the
        // loopback default with no signal at all.
        assert_eq!(
            capture_warnings(|| {
                resolve_searxng_instance_url("searxng", Some("192.168.1.50:8888"));
            })
            .len(),
            1
        );

        // Neither a supported unset value nor a valid URL may warn: a noisy
        // warning would train the operator to ignore the one that matters.
        assert!(capture_warnings(|| {
            resolve_searxng_instance_url("searxng", None);
        })
        .is_empty());
        assert!(capture_warnings(|| {
            resolve_searxng_instance_url("searxng", Some("http://127.0.0.1:18893"));
        })
        .is_empty());

        // The resolution itself is unchanged: unset and malformed both land on
        // the documented loopback default, a valid value is honoured verbatim.
        let loopback = Url::parse("http://127.0.0.1:8888").unwrap();
        assert_eq!(resolve_searxng_instance_url("searxng", None), loopback);
        assert_eq!(
            resolve_searxng_instance_url("searxng", Some("not-a-url")),
            loopback
        );
        assert_eq!(
            resolve_searxng_instance_url("searxng", Some("http://127.0.0.1:18893")),
            Url::parse("http://127.0.0.1:18893").unwrap()
        );
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
    fn answer_without_a_url_is_dropped_not_promoted_with_an_empty_url() {
        // A promoted answer must be openable. Inserting it with `url: ""`
        // handed `normalize` an item it rejects with an `invalid_url`
        // degradation, flipping a clean response to `partial_success`.
        let response = HttpResponse {
            status: 200,
            headers: BTreeMap::new(),
            body: br#"{
                "query": "rust",
                "results": [],
                "answers": [
                    {"answer": "no url answer"},
                    {"answer": "null url answer", "url": null},
                    {"answer": "empty url answer", "url": ""},
                    {"answer": "kept", "url": "https://example.com/"}
                ],
                "unresponsive_engines": []
            }"#
            .to_vec(),
        };
        let result = parse_response(response, FilterUse::default()).unwrap();
        assert_eq!(result.results.len(), 1);
        assert_eq!(result.results[0].url, "https://example.com/");
        assert_eq!(result.results[0].provider_rank, Rank::new(1).ok());
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
    fn a_url_only_answer_does_not_discard_the_whole_response() {
        // SearXNG emits answers whose only payload is a URL (infobox/entity
        // cards): `{"url": "..."}` with no `answer` key. Verified against serde
        // 1.0.229: a missing key yields `None` for `Option<T>` even without
        // `#[serde(default)]`, so the whole body still deserializes. This test
        // pins that guarantee, because it is what keeps one promotional element
        // from costing every usable result — a `#[serde(deny_unknown_fields)]`
        // or a switch to a non-Option field would reinstate the all-or-nothing
        // failure with no other test noticing.
        let response = HttpResponse {
            status: 200,
            headers: BTreeMap::new(),
            body: br#"{
                "query": "rust",
                "results": [{"title": "Kept", "url": "https://example.com/"}],
                "answers": [{"url": "https://example.com/infobox"}],
                "unresponsive_engines": []
            }"#
            .to_vec(),
        };
        let result = parse_response(response, FilterUse::default()).unwrap();
        assert_eq!(
            result.results.len(),
            2,
            "the url-only answer is promoted with a null title; the real result must survive too"
        );
        let promoted = result
            .results
            .iter()
            .find(|item| item.url == "https://example.com/infobox")
            .expect("the url-only answer keeps its openable URL");
        // `title` is nullable in the public contract (`required` + `string|null`),
        // and dedupe prefers a title-bearing representative, so a null title on a
        // URL-only answer is tolerated, not degraded.
        assert_eq!(promoted.title, None);
        assert!(result
            .results
            .iter()
            .any(|item| item.url == "https://example.com/"));
    }

    #[test]
    fn preserves_ignored_and_approximated_filters_in_the_provider_result() {
        // Reproduces F3: `translated_query` classifies `site` as
        // approximated and `language`/`region` as ignored, but
        // `parse_response` used to discard that classification and always
        // report empty filter lists, even though the schema (and every
        // other provider) carries it through to `ProviderResult`.
        let (_query, filters) = translated_query(&plan("rust site:docs.rs lang:es region:MX"));
        assert!(filters.approximated.contains(&"site".into()));
        assert!(filters.ignored.contains(&"language".into()));
        assert!(filters.ignored.contains(&"region".into()));

        let response = HttpResponse {
            status: 200,
            headers: BTreeMap::new(),
            body: br#"{"query": "rust", "results": [], "unresponsive_engines": []}"#.to_vec(),
        };
        let result = parse_response(response, filters).unwrap();

        assert!(result.accepted_filters.is_empty());
        assert!(result.ignored_filters.contains(&"language".into()));
        assert!(result.ignored_filters.contains(&"region".into()));
        assert!(result.approximated_filters.contains(&"site".into()));
    }

    #[test]
    fn marks_partial_when_engines_are_unresponsive() {
        // Real shape: a list of `[engine_name, error_type]` pairs, not a
        // list of plain strings — see the field's doc comment.
        let response = HttpResponse {
            status: 200,
            headers: BTreeMap::new(),
            body: br#"{
                "query": "rust",
                "results": [{"title": "Rust", "url": "https://rust-lang.org/", "content": "ok"}],
                "unresponsive_engines": [["example_engine", "timeout"], ["google", "too many requests"]]
            }"#
            .to_vec(),
        };
        let result = parse_response(response, FilterUse::default()).unwrap();
        assert_eq!(result.status, ProviderExecutionStatus::Partial);
    }

    /// Regression for a real `searxng/searxng:latest` response (captured
    /// 2026-08-15): extra per-result fields this parser never mapped
    /// (`engines`, `positions`, `parsed_url`, `score`, …) are silently
    /// ignored as always, and `unresponsive_engines` — the one field whose
    /// real shape didn't match this struct — now parses instead of failing
    /// with "invalid type: sequence, expected a string".
    #[test]
    fn parses_a_real_instance_response_including_unmapped_extra_fields() {
        let response = HttpResponse {
            status: 200,
            headers: BTreeMap::new(),
            body: br#"{
                "query": "rust",
                "results": [{
                    "template": "default.html",
                    "title": "Rust Programming Language",
                    "url": "https://www.rust-lang.org/",
                    "content": "A language empowering everyone.",
                    "img_src": "", "iframe_src": "", "audio_src": "", "thumbnail": "",
                    "publishedDate": null, "pubdate": "", "length": null,
                    "views": "", "author": "", "metadata": "", "priority": "",
                    "engines": ["google", "example_engine"],
                    "open_group": false, "close_group": false,
                    "positions": [1, 3],
                    "score": 4.5,
                    "category": "general",
                    "engine": "google",
                    "parsed_url": ["https", "www.rust-lang.org", "/", "", "", ""]
                }],
                "answers": [],
                "corrections": [],
                "infoboxes": [],
                "suggestions": [],
                "unresponsive_engines": [["example_engine", "too many requests"], ["startpage", "Suspended: CAPTCHA"]]
            }"#
            .to_vec(),
        };
        let result = parse_response(response, FilterUse::default()).unwrap();
        assert_eq!(result.status, ProviderExecutionStatus::Partial);
        assert_eq!(result.results.len(), 1);
        assert_eq!(result.results[0].url, "https://www.rust-lang.org/");
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
    fn malformed_json_is_typed_invalid_response_not_a_panic() {
        let response = HttpResponse {
            status: 200,
            headers: BTreeMap::new(),
            body: b"{\"results\": [".to_vec(),
        };
        let error = parse_response(response, FilterUse::default()).unwrap_err();
        assert_eq!(error.kind, ProviderErrorKind::InvalidResponse);
    }

    #[test]
    fn result_missing_a_url_is_dropped_like_marginalia() {
        // A result the caller cannot open is not a result. The old behavior
        // (`url: ""`) let the malformed item reach the pipeline, where it was
        // rejected per item, inflating the reported count and degrading the
        // aggregated status. The adapter now drops it, matching Marginalia.
        let response = HttpResponse {
            status: 200,
            headers: BTreeMap::new(),
            body: br#"{"results": [{"title": "Rust"}, {"title": "Valid", "url": "https://rust-lang.org/"}], "answers": [], "unresponsive_engines": []}"#
                .to_vec(),
        };
        let result = parse_response(response, FilterUse::default()).unwrap();
        assert_eq!(result.results.len(), 1);
        assert_eq!(result.results[0].url, "https://rust-lang.org/");
        assert_eq!(result.results[0].provider_rank, Rank::new(1).ok());
    }

    #[test]
    fn results_with_null_or_empty_url_are_dropped() {
        let response = HttpResponse {
            status: 200,
            headers: BTreeMap::new(),
            body: br#"{"results": [
                {"title": "Null", "url": null},
                {"title": "Empty", "url": ""},
                {"title": "Kept", "url": "https://example.com/"}
            ], "answers": [], "unresponsive_engines": []}"#
                .to_vec(),
        };
        let result = parse_response(response, FilterUse::default()).unwrap();
        assert_eq!(result.results.len(), 1);
        assert_eq!(result.results[0].url, "https://example.com/");
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
