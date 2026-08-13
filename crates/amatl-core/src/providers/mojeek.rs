use super::{
    HttpRequest, HttpResponse, HttpTransport, Provider, ProviderAvailability, ProviderContext,
};
use crate::model::{
    ProviderCapabilities, ProviderError, ProviderErrorKind, ProviderExecutionStatus, ProviderItem,
    ProviderResult, Rank, SearchPlan, SCHEMA_VERSION,
};
use async_trait::async_trait;
use serde::Deserialize;
use std::collections::BTreeSet;
use std::sync::Arc;
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;
use url::Url;

const ENDPOINT: &str = "https://www.mojeek.com/search";

pub struct MojeekProvider {
    api_key: Option<String>,
    enabled: bool,
    approved: bool,
    supported_filters: BTreeSet<String>,
    transport: Arc<dyn HttpTransport>,
}

impl MojeekProvider {
    pub fn new(
        api_key: Option<String>,
        enabled: bool,
        approved: bool,
        supported_filters: Vec<String>,
        transport: Arc<dyn HttpTransport>,
    ) -> Self {
        Self {
            api_key,
            enabled,
            approved,
            supported_filters: supported_filters.into_iter().collect(),
            transport,
        }
    }

    fn supports(&self, capability: &str) -> bool {
        self.supported_filters.contains(capability)
    }

    fn request(
        &self,
        plan: &SearchPlan,
        timeout_ms: u64,
    ) -> Result<(HttpRequest, FilterUse), ProviderError> {
        let key = self.api_key.as_ref().ok_or_else(|| {
            error(
                ProviderErrorKind::Auth,
                "Mojeek credential is unavailable",
                None,
            )
        })?;
        let query = &plan.query;
        let mut url = Url::parse(ENDPOINT).map_err(|_| {
            error(
                ProviderErrorKind::InvalidResponse,
                "Mojeek endpoint configuration is invalid",
                None,
            )
        })?;
        let mut accepted = vec![];
        let mut ignored = vec![];
        let mut approximated = vec![];
        {
            let mut pairs = url.query_pairs_mut();
            let mut search = query.normalized_query.clone();
            for term in &query.quoted_terms {
                search.push_str(&format!(" \"{term}\""));
            }
            pairs
                .append_pair("api_key", key)
                .append_pair("q", search.trim())
                .append_pair("fmt", "json")
                .append_pair("t", "10");
            if !query.excluded_terms.is_empty() && self.supports("excluded_terms") {
                pairs.append_pair("qm", &query.excluded_terms.join(" "));
                accepted.push("excluded_terms".into());
            } else if !query.excluded_terms.is_empty() {
                ignored.push("excluded_terms".into());
            }
            if !query.domains.is_empty() && self.supports("site") {
                pairs.append_pair("fi", &query.domains.join(","));
                accepted.push("site".into());
            } else if !query.domains.is_empty() {
                ignored.push("site".into());
            }
            if !query.excluded_domains.is_empty() && self.supports("excluded_site") {
                pairs.append_pair("fe", &query.excluded_domains.join(","));
                accepted.push("excluded_site".into());
            } else if !query.excluded_domains.is_empty() {
                ignored.push("excluded_site".into());
            }
            if let Some(from) = query
                .date_from
                .as_ref()
                .filter(|_| self.supports("time_range"))
            {
                pairs.append_pair("since", &from.replace('-', ""));
                accepted.push("date_from".into());
            } else if query.date_from.is_some() {
                ignored.push("date_from".into());
            }
            if let Some(to) = query
                .date_to
                .as_ref()
                .filter(|_| self.supports("time_range"))
            {
                pairs.append_pair("before", &to.replace('-', ""));
                accepted.push("date_to".into());
            } else if query.date_to.is_some() {
                ignored.push("date_to".into());
            }
            if let Some(region) = query.region.as_ref().filter(|_| self.supports("region")) {
                pairs.append_pair("rb", region).append_pair("rbb", "10");
                approximated.push("region".into());
            } else if query.region.is_some() {
                ignored.push("region".into());
            }
            if let Some(language) = query
                .language
                .as_ref()
                .filter(|_| self.supports("language"))
            {
                if language.len() == 2 {
                    pairs
                        .append_pair("lb", &language.to_ascii_uppercase())
                        .append_pair("lbb", "100");
                    approximated.push("language".into());
                } else {
                    ignored.push("language".into());
                }
            } else if query.language.is_some() {
                ignored.push("language".into());
            }
        }
        if !query.file_types.is_empty() {
            ignored.push("filetype".into());
        }
        Ok((
            HttpRequest {
                url,
                headers: vec![("accept".into(), "application/json".into())],
                timeout_ms,
            },
            FilterUse {
                accepted,
                ignored,
                approximated,
            },
        ))
    }
}

#[async_trait]
impl Provider for MojeekProvider {
    fn name(&self) -> &str {
        "mojeek"
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            schema_version: SCHEMA_VERSION.into(),
            pagination: true,
            language: self.supports("language"),
            region: self.supports("region"),
            time_range: self.supports("time_range"),
            site_filter: self.supports("site"),
            file_filter: false,
            news: false,
            code: false,
            docs: false,
            academic: false,
            authentication: true,
            estimated_cost: Some(3),
        }
    }

    fn availability(&self) -> ProviderAvailability {
        if !self.enabled {
            ProviderAvailability::Unavailable {
                code: "provider_disabled".into(),
                message: "Mojeek is disabled by configuration".into(),
            }
        } else if !self.approved {
            ProviderAvailability::Unavailable {
                code: "provider_not_approved".into(),
                message: "Mojeek governance approval is required".into(),
            }
        } else if self.api_key.as_deref().is_none_or(str::is_empty) {
            ProviderAvailability::Unavailable {
                code: "provider_credential_missing".into(),
                message: "Mojeek credential is unavailable".into(),
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
                "Mojeek network request failed",
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

#[derive(Deserialize)]
struct Envelope {
    response: Body,
}
#[derive(Deserialize)]
struct Body {
    status: String,
    #[serde(default)]
    results: Vec<Item>,
}
#[derive(Deserialize)]
struct Item {
    url: String,
    title: Option<String>,
    desc: Option<String>,
    pdate: Option<i64>,
}

fn parse_response(
    response: HttpResponse,
    filters: FilterUse,
) -> Result<ProviderResult, ProviderError> {
    if response.status != 200 {
        return Err(status_error(&response));
    }
    let envelope: Envelope = serde_json::from_slice(&response.body).map_err(|_| {
        error(
            ProviderErrorKind::InvalidResponse,
            "Mojeek returned invalid JSON",
            None,
        )
    })?;
    if !envelope.response.status.eq_ignore_ascii_case("ok") {
        let kind = if envelope
            .response
            .status
            .to_ascii_lowercase()
            .contains("limit")
        {
            ProviderErrorKind::Quota
        } else {
            ProviderErrorKind::InvalidResponse
        };
        return Err(error(
            kind,
            "Mojeek returned an unsuccessful response",
            None,
        ));
    }
    let results = envelope
        .response
        .results
        .into_iter()
        .enumerate()
        .map(|(index, item)| ProviderItem {
            title: item.title,
            url: item.url,
            provider_rank: Rank::new(index as u32 + 1).ok(),
            snippet: item.desc,
            result_type: None,
            published_at: item.pdate.and_then(|timestamp| {
                OffsetDateTime::from_unix_timestamp(timestamp)
                    .ok()
                    .and_then(|date| date.format(&Rfc3339).ok())
            }),
            author: None,
            language: None,
            file_type: None,
            thumbnail: None,
            metadata: Default::default(),
        })
        .collect();
    Ok(ProviderResult {
        schema_version: SCHEMA_VERSION.into(),
        provider: "mojeek".into(),
        status: ProviderExecutionStatus::Success,
        results,
        accepted_filters: filters.accepted,
        ignored_filters: filters.ignored,
        approximated_filters: filters.approximated,
        errors: vec![],
    })
}

fn status_error(response: &HttpResponse) -> ProviderError {
    let kind = match response.status {
        401 | 403 => ProviderErrorKind::Auth,
        429 => ProviderErrorKind::RateLimit,
        402 => ProviderErrorKind::Quota,
        500..=599 => ProviderErrorKind::Unavailable,
        _ => ProviderErrorKind::InvalidResponse,
    };
    let retry = response
        .headers
        .get("retry-after")
        .and_then(|value| value.parse::<u64>().ok())
        .map(|seconds| seconds.saturating_mul(1000));
    error(kind, "Mojeek returned an unsuccessful HTTP status", retry)
}

fn error(kind: ProviderErrorKind, message: &str, retry_after_ms: Option<u64>) -> ProviderError {
    ProviderError {
        schema_version: SCHEMA_VERSION.into(),
        provider: "mojeek".into(),
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
                selected_providers: vec!["mojeek".into()],
                provider_budget_requests: BTreeMap::from([("mojeek".into(), 1)]),
                debug_reasons: vec![],
            },
            &mut budget,
        )
    }

    #[test]
    fn maps_supported_and_approximated_filters() {
        let provider = MojeekProvider::new(
            Some("top-secret".into()),
            true,
            true,
            vec![
                "site".into(),
                "excluded_site".into(),
                "language".into(),
                "region".into(),
            ],
            Arc::new(super::super::http::ReqwestTransport::new(1024).unwrap()),
        );
        let (request, filters) = provider
            .request(
                &plan("rust site:docs.rs -site:bad.test lang:es region:MX filetype:pdf"),
                100,
            )
            .unwrap();
        let url = request.url.to_string();
        assert!(url.contains("fi=docs.rs"));
        assert!(url.contains("fe=bad.test"));
        assert!(filters.approximated.contains(&"language".into()));
        assert!(filters.ignored.contains(&"filetype".into()));
        assert!(!request.sanitized_url().to_string().contains("top-secret"));
    }

    #[test]
    fn does_not_simulate_filters_missing_from_the_contracted_plan() {
        let provider = MojeekProvider::new(
            Some("top-secret".into()),
            true,
            true,
            vec![],
            Arc::new(super::super::http::ReqwestTransport::new(1024).unwrap()),
        );
        let (request, filters) = provider
            .request(&plan("rust site:docs.rs lang:es region:MX"), 100)
            .unwrap();
        let url = request.sanitized_url().to_string();
        assert!(!url.contains("fi="));
        assert!(!url.contains("lb="));
        assert!(!url.contains("rb="));
        assert!(filters.ignored.contains(&"site".into()));
        assert!(filters.ignored.contains(&"language".into()));
        assert!(filters.ignored.contains(&"region".into()));
    }

    #[test]
    fn parses_official_json_shape() {
        let response = HttpResponse { status: 200, headers: BTreeMap::new(), body: br#"{"response":{"status":"OK","results":[{"url":"https://www.mojeek.com/","title":"Mojeek","desc":"Independent search"}]}}"#.to_vec() };
        let result = parse_response(response, FilterUse::default()).unwrap();
        assert_eq!(result.results.len(), 1);
        assert_eq!(result.results[0].provider_rank, Rank::new(1).ok());
    }

    #[test]
    fn daily_limit_is_typed_quota_without_upstream_body() {
        let response = HttpResponse {
            status: 200,
            headers: BTreeMap::new(),
            body: br#"{"response":{"status":"ERROR: Daily Limit Reached","results":[]}}"#.to_vec(),
        };
        let error = parse_response(response, FilterUse::default()).unwrap_err();
        assert_eq!(error.kind, ProviderErrorKind::Quota);
        assert!(!error.message.contains("Daily"));
    }
}
