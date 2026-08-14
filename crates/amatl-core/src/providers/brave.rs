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

const ENDPOINT: &str = "https://api.search.brave.com/res/v1/web/search";

pub struct BraveProvider {
    token: Option<String>,
    enabled: bool,
    approved: bool,
    transport: Arc<dyn HttpTransport>,
}

impl BraveProvider {
    pub fn new(
        token: Option<String>,
        enabled: bool,
        approved: bool,
        transport: Arc<dyn HttpTransport>,
    ) -> Self {
        Self {
            token,
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
        let token = self.token.as_ref().ok_or_else(|| {
            error(
                ProviderErrorKind::Auth,
                "Brave credential is unavailable",
                None,
            )
        })?;
        let mut url = Url::parse(ENDPOINT).map_err(|_| {
            error(
                ProviderErrorKind::InvalidResponse,
                "Brave endpoint configuration is invalid",
                None,
            )
        })?;
        let (query, filters) = translated_query(plan);
        {
            let mut pairs = url.query_pairs_mut();
            pairs.append_pair("q", &query).append_pair("count", "20");
            if let Some(region) = &plan.query.region {
                pairs.append_pair("country", region);
            }
            if let Some(language) = &plan.query.language {
                pairs.append_pair("search_lang", language);
            }
            if let (Some(from), Some(to)) = (&plan.query.date_from, &plan.query.date_to) {
                pairs.append_pair("freshness", &format!("{from}to{to}"));
            }
        }
        Ok((
            HttpRequest::get(
                url,
                vec![
                    ("accept".into(), "application/json".into()),
                    ("cache-control".into(), "no-cache".into()),
                    ("x-subscription-token".into(), token.clone()),
                ],
                timeout_ms,
            ),
            filters,
        ))
    }
}

#[async_trait]
impl Provider for BraveProvider {
    fn name(&self) -> &str {
        "brave"
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            schema_version: SCHEMA_VERSION.into(),
            pagination: true,
            language: true,
            region: true,
            time_range: true,
            site_filter: true,
            file_filter: true,
            news: true,
            code: false,
            docs: false,
            academic: false,
            authentication: true,
            estimated_cost: Some(5),
        }
    }

    fn availability(&self) -> ProviderAvailability {
        if !self.enabled {
            ProviderAvailability::Unavailable {
                code: "provider_disabled".into(),
                message: "Brave is disabled by configuration".into(),
            }
        } else if !self.approved {
            ProviderAvailability::Unavailable {
                code: "provider_not_approved".into(),
                message: "Brave governance approval is required".into(),
            }
        } else if self.token.as_deref().is_none_or(str::is_empty) {
            ProviderAvailability::Unavailable {
                code: "provider_credential_missing".into(),
                message: "Brave credential is unavailable".into(),
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
                "Brave network request failed",
                None,
            )
        })?;
        parse_response(response, filters)
    }
}

#[derive(Default)]
struct FilterUse {
    accepted: Vec<String>,
    ignored: Vec<String>,
    approximated: Vec<String>,
}

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
    terms.extend(
        query
            .file_types
            .iter()
            .map(|kind| format!("filetype:{kind}")),
    );
    let mut accepted = vec![];
    if !query.domains.is_empty() || !query.excluded_domains.is_empty() {
        accepted.push("site".into());
    }
    if !query.file_types.is_empty() {
        accepted.push("filetype".into());
    }
    if query.language.is_some() {
        accepted.push("language".into());
    }
    if query.region.is_some() {
        accepted.push("region".into());
    }
    let mut ignored = vec![];
    if query.date_from.is_some() ^ query.date_to.is_some() {
        ignored.push("time_range".into());
    }
    if query.date_from.is_some() && query.date_to.is_some() {
        accepted.push("time_range".into());
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

#[derive(Deserialize)]
struct BraveEnvelope {
    web: Option<BraveWeb>,
}
#[derive(Deserialize)]
struct BraveWeb {
    #[serde(default)]
    results: Vec<BraveItem>,
}
#[derive(Deserialize)]
struct BraveItem {
    title: Option<String>,
    url: String,
    description: Option<String>,
}

fn parse_response(
    response: HttpResponse,
    filters: FilterUse,
) -> Result<ProviderResult, ProviderError> {
    if response.status != 200 {
        return Err(status_error("Brave", &response));
    }
    let envelope: BraveEnvelope = serde_json::from_slice(&response.body).map_err(|_| {
        error(
            ProviderErrorKind::InvalidResponse,
            "Brave returned invalid JSON",
            None,
        )
    })?;
    let items = envelope.web.map_or_else(Vec::new, |web| web.results);
    let results = items
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
            metadata: Default::default(),
        })
        .collect();
    Ok(ProviderResult {
        schema_version: SCHEMA_VERSION.into(),
        provider: "brave".into(),
        status: ProviderExecutionStatus::Success,
        results,
        accepted_filters: filters.accepted,
        ignored_filters: filters.ignored,
        approximated_filters: filters.approximated,
        errors: vec![],
    })
}

fn status_error(provider_label: &str, response: &HttpResponse) -> ProviderError {
    let (kind, message) = match response.status {
        401 | 403 => (ProviderErrorKind::Auth, "provider authentication rejected"),
        429 => (ProviderErrorKind::RateLimit, "provider rate limit reached"),
        402 => (ProviderErrorKind::Quota, "provider quota exhausted"),
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
        provider: "brave".into(),
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
                selected_providers: vec!["brave".into()],
                provider_budget_requests: BTreeMap::from([("brave".into(), 1)]),
                debug_reasons: vec![],
            },
            &mut budget,
        )
    }

    #[test]
    fn maps_structured_filters_without_exposing_token() {
        let provider = BraveProvider::new(
            Some("top-secret".into()),
            true,
            true,
            Arc::new(super::super::http::ReqwestTransport::new(1024).unwrap()),
        );
        let (request, filters) = provider
            .request(
                &plan("rust site:docs.rs lang:es region:MX after:2025-01-01 before:2026-01-01"),
                100,
            )
            .unwrap();
        let url = request.url.to_string();
        assert!(url.contains("site%3Adocs.rs"));
        assert!(url.contains("search_lang=es"));
        assert!(!url.contains("top-secret"));
        assert!(filters.accepted.contains(&"time_range".into()));
    }

    #[test]
    fn parses_fixture_and_preserves_native_order() {
        let response = HttpResponse { status: 200, headers: BTreeMap::new(), body: br#"{"web":{"results":[{"title":"One","url":"https://one.example/","description":"First"},{"title":"Two","url":"https://two.example/"}]}}"#.to_vec() };
        let result = parse_response(response, FilterUse::default()).unwrap();
        assert_eq!(result.results[0].provider_rank, Rank::new(1).ok());
        assert_eq!(result.results[1].provider_rank, Rank::new(2).ok());
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
}
