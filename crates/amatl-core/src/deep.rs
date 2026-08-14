use crate::budget::DeepBudget;
use crate::document_cache::DocumentCache;
use crate::extract::{ExtractError, Extractor};
use crate::fetch::{FetchError, FetchRequest, Fetcher};
use crate::gaps::{GapAnalyzer, SubQueryExecutor};
use crate::model::{
    CanonicalUrl, CompositeError, DeepResponse, Degradation, Document, DocumentStatus, FetchMethod,
    OriginalUrl, SearchResult, SearchStatus, SubQuery, SubQueryStatus, SCHEMA_VERSION,
};
use crate::ranking_v2::{disabled_output, rejected_output, RankingV2Engine};
use crate::render::RendererPool;
use crate::robots::{RobotsCache, RobotsDecision};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::Arc;
use std::time::Instant;

#[derive(Clone, Debug)]
pub struct DeepCandidate {
    pub result: SearchResult,
    pub storage_rights: bool,
}

#[derive(Clone, Debug)]
pub struct DeepRequest {
    pub query: crate::model::Query,
    pub search_plan: crate::model::SearchPlan,
    pub candidates: Vec<DeepCandidate>,
}

pub struct DeepOrchestrator {
    budget: DeepBudget,
    fetcher: Arc<dyn Fetcher>,
    extractor: Arc<dyn Extractor>,
    renderer: RendererPool,
    cache: Option<DocumentCache>,
    timeout_ms: u64,
    per_fetch_bytes: u64,
    max_redirects: u32,
    top_k: usize,
    max_depth: u8,
    ranking_v2: Option<RankingV2Engine>,
    gap_analyzer: Option<GapAnalyzer>,
    subquery_executor: Option<Arc<dyn SubQueryExecutor>>,
    /// Correlates every outbound document fetch with the originating request.
    request_id: Option<String>,
    /// Consulted for links AMATL discovers itself. `None` disables the check,
    /// which only an operator may choose.
    robots: Option<RobotsCache>,
}

impl DeepOrchestrator {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        budget: DeepBudget,
        fetcher: Arc<dyn Fetcher>,
        extractor: Arc<dyn Extractor>,
        renderer: RendererPool,
        cache: Option<DocumentCache>,
        timeout_ms: u64,
        per_fetch_bytes: u64,
        max_redirects: u32,
        top_k: usize,
        max_depth: u8,
    ) -> Self {
        Self {
            budget,
            fetcher,
            extractor,
            renderer,
            cache,
            timeout_ms,
            per_fetch_bytes,
            max_redirects,
            top_k,
            max_depth,
            ranking_v2: None,
            gap_analyzer: None,
            subquery_executor: None,
            request_id: None,
            robots: None,
        }
    }

    /// Consult `robots.txt` before fetching any link discovered by the crawl.
    ///
    /// URLs that came from Search are not gated: those are requested by the
    /// user, not discovered by AMATL.
    pub fn with_robots(mut self, robots: RobotsCache) -> Self {
        self.robots = Some(robots);
        self
    }

    /// Correlate every fetch this orchestrator performs with the caller's
    /// request id.
    pub fn with_request_id(mut self, request_id: Option<String>) -> Self {
        self.request_id = request_id;
        self
    }

    pub fn with_ranking_v2(mut self, engine: RankingV2Engine) -> Self {
        self.ranking_v2 = Some(engine);
        self
    }

    pub fn with_gap_analyzer(mut self, analyzer: GapAnalyzer) -> Self {
        self.gap_analyzer = Some(analyzer);
        self
    }

    pub fn with_subquery_executor(mut self, executor: Arc<dyn SubQueryExecutor>) -> Self {
        self.subquery_executor = Some(executor);
        self
    }

    pub async fn enrich(&mut self, request: DeepRequest) -> DeepResponse {
        let started = Instant::now();
        let deadline = tokio::time::Instant::now()
            + std::time::Duration::from_millis(self.budget.snapshot().deadline_ms);
        let mut response = DeepResponse {
            schema_version: SCHEMA_VERSION.into(),
            query: request.query.raw_query.clone(),
            documents: vec![],
            errors: vec![],
            degradations: vec![],
            evidence: vec![],
            evidence_v2: vec![],
            ranking_v2: disabled_output(),
            gaps: vec![],
            subqueries: vec![],
            elapsed_ms: 0,
        };
        if request.search_plan.query.normalized_query != request.query.normalized_query {
            response.errors.push(CompositeError {
                code: "deep_plan_query_mismatch".into(),
                message: "Deep Query does not match SearchPlan".into(),
                providers: vec![],
                recoverable: false,
            });
            return response;
        }
        if request.candidates.is_empty() {
            response.degradations.push(degradation(
                "deep_no_candidates",
                "No SearchResult candidates were available",
            ));
            return response;
        }
        if !self.renderer.available() {
            response.degradations.push(degradation(
                "renderer_unavailable",
                "Optional Chromium renderer is unavailable; Deep continues without it",
            ));
        }
        let mut original_ranks = request
            .candidates
            .iter()
            .map(|candidate| (search_result_id(&candidate.result), candidate.result.rank))
            .collect::<BTreeMap<_, _>>();
        let mut known_urls = request
            .candidates
            .iter()
            .map(|candidate| candidate.result.canonical_url.0.as_str().to_owned())
            .collect::<BTreeSet<_>>();
        let mut queue: VecDeque<(DeepCandidate, u8)> = request
            .candidates
            .into_iter()
            .take(self.top_k)
            .map(|candidate| (candidate, 0))
            .collect();
        let mut visited: BTreeSet<String> = queue
            .iter()
            .map(|(candidate, _)| candidate.result.canonical_url.0.as_str().to_owned())
            .collect();
        while let Some((candidate, depth)) = queue.pop_front() {
            let now = tokio::time::Instant::now();
            if now >= deadline {
                response.degradations.push(degradation(
                    "time_exhausted",
                    "Deep global deadline was exhausted",
                ));
                break;
            }
            // Depth 0 is what the user asked for; anything deeper is our own
            // discovery and needs the origin's consent first.
            if depth > 0 {
                if let Some(robots) = &self.robots {
                    match robots
                        .decide(&candidate.result.canonical_url.0, self.request_id.clone())
                        .await
                    {
                        RobotsDecision::Allowed { crawl_delay_ms } if crawl_delay_ms > 0 => {
                            let wait = std::time::Duration::from_millis(crawl_delay_ms);
                            // Politeness never outlives the Deep deadline.
                            if tokio::time::Instant::now() + wait >= deadline {
                                response.degradations.push(degradation(
                                    "robots_crawl_delay_too_long",
                                    "Declared crawl delay exceeded the remaining Deep deadline",
                                ));
                                continue;
                            }
                            tokio::time::sleep(wait).await;
                        }
                        RobotsDecision::Allowed { .. } => {}
                        refusal => {
                            response.degradations.push(degradation(
                                refusal.as_str(),
                                "Discovered link was not crawled: the origin's robots.txt refused it or could not be read",
                            ));
                            continue;
                        }
                    }
                }
            }
            let remaining = match self.budget.reserve_fetch() {
                Ok(value) => value,
                Err(cause) => {
                    response.degradations.push(degradation(
                        &cause.to_string(),
                        "Deep resource budget exhausted",
                    ));
                    break;
                }
            };
            let limit = remaining.min(self.per_fetch_bytes);
            let redirect_limit = self
                .budget
                .snapshot()
                .remaining_redirects
                .min(self.max_redirects);
            let fetch_timeout = deadline
                .saturating_duration_since(now)
                .as_millis()
                .min(u128::from(self.timeout_ms)) as u64;
            let revalidation_document = match &self.cache {
                Some(cache) => cache
                    .latest(
                        candidate.result.canonical_url.0.as_str(),
                        self.extractor.version(),
                    )
                    .await
                    .filter(|document| document.content.is_some()),
                None => None,
            };
            let mut request_headers = default_headers();
            if let Some(document) = &revalidation_document {
                if let Some(etag) = document.metadata.get("http_etag") {
                    request_headers.insert("if-none-match".into(), etag.clone());
                }
                if let Some(last_modified) = document.metadata.get("http_last_modified") {
                    request_headers.insert("if-modified-since".into(), last_modified.clone());
                }
            }
            let fetch = self
                .fetcher
                .fetch(FetchRequest {
                    url: candidate.result.canonical_url.0.clone(),
                    timeout_ms: fetch_timeout,
                    max_bytes: limit,
                    max_redirects: redirect_limit,
                    headers: request_headers,
                    request_id: self.request_id.clone(),
                })
                .await;
            let fetched = match fetch {
                Ok(value) => value,
                Err(error) => {
                    if matches!(
                        error,
                        FetchError::EgressDenied
                            | FetchError::BlockedUrl(_)
                            | FetchError::AddressBlocked
                            | FetchError::HeaderBlocked
                    ) {
                        self.budget.release_fetch();
                    }
                    response.errors.push(fetch_error(&candidate.result, &error));
                    response.degradations.push(degradation(
                        fetch_code(&error),
                        "Fetch failed; the SearchResult remains valid",
                    ));
                    continue;
                }
            };
            if let Err(cause) = self
                .budget
                .consume_fetch(fetched.size, fetched.redirect_chain.len() as u32)
            {
                response.degradations.push(degradation(
                    &cause.to_string(),
                    "Fetch result exceeded remaining Deep budget",
                ));
                continue;
            }
            if fetched.status == 304 {
                if let Some(mut document) = revalidation_document {
                    document.final_url = fetched.final_url;
                    document.retrieved_at = fetched.retrieved_at;
                    response.documents.push(document);
                    continue;
                }
                response.degradations.push(degradation(
                    "unexpected_not_modified",
                    "Origin returned 304 without a reusable cached Document",
                ));
                continue;
            }
            let mut content_hash = hex_digest(&fetched.body);
            let result_id = search_result_id(&candidate.result);
            if let Some(cache) = &self.cache {
                if let Some(document) = cache
                    .get(
                        candidate.result.canonical_url.0.as_str(),
                        &content_hash,
                        self.extractor.version(),
                    )
                    .await
                {
                    if document.content.is_some() {
                        response.documents.push(document);
                        continue;
                    }
                }
            }
            let mut extraction =
                tokio::time::timeout_at(deadline, self.extractor.extract(&fetched.body))
                    .await
                    .unwrap_or(Err(ExtractError::Timeout));
            let mut fetch_method = FetchMethod::Http;
            let mut final_url = fetched.final_url;
            let mut document_size = fetched.size;
            if extraction.is_err() && self.renderer.available() {
                match self.budget.reserve_browser() {
                    Ok(()) => {
                        match tokio::time::timeout_at(deadline, self.renderer.render(&final_url.0))
                            .await
                            .unwrap_or(Err(crate::render::RenderError::Timeout))
                        {
                            Ok(rendered) => {
                                if self
                                    .budget
                                    .consume_fetch(rendered.dom.len() as u64, rendered.redirects)
                                    .is_ok()
                                {
                                    final_url = rendered.final_url;
                                    content_hash = hex_digest(&rendered.dom);
                                    document_size = rendered.dom.len() as u64;
                                    extraction = tokio::time::timeout_at(
                                        deadline,
                                        self.extractor.extract(&rendered.dom),
                                    )
                                    .await
                                    .unwrap_or(Err(ExtractError::Timeout));
                                    fetch_method = FetchMethod::Rendered;
                                } else {
                                    response.degradations.push(degradation(
                                        "renderer_blocked",
                                        "Rendered output exceeded the remaining Deep budget",
                                    ));
                                }
                            }
                            Err(error) => {
                                if error == crate::render::RenderError::Unavailable {
                                    self.budget.release_browser();
                                }
                                response.degradations.push(degradation(
                                    &error.to_string(),
                                    "Optional renderer failed; the superficial Document is retained",
                                ));
                            }
                        }
                    }
                    Err(cause) => response
                        .degradations
                        .push(degradation(&cause.to_string(), "Browser budget exhausted")),
                }
            }
            let (status, content, title, author, published_at, mut metadata, extractor_used) =
                match extraction {
                    Ok(value) => (
                        DocumentStatus::Enriched,
                        Some(value.content),
                        value.title,
                        value.author,
                        value.published_at,
                        value.metadata,
                        Some(value.extractor_used),
                    ),
                    Err(error) => {
                        response.degradations.push(degradation(extract_code(&error), "Advanced extraction is unavailable; a superficial Document was retained"));
                        (
                            DocumentStatus::Superficial,
                            None,
                            candidate.result.title.clone(),
                            None,
                            candidate.result.published_at.clone(),
                            BTreeMap::new(),
                            None,
                        )
                    }
                };
            if let Some(etag) = fetched.headers_safe.get("etag") {
                metadata.insert("http_etag".into(), etag.clone());
            }
            if let Some(last_modified) = fetched.headers_safe.get("last-modified") {
                metadata.insert("http_last_modified".into(), last_modified.clone());
            }
            let document = Document {
                schema_version: SCHEMA_VERSION.into(),
                search_result_id: result_id,
                original_url: OriginalUrl(candidate.result.original_url.0.clone()),
                canonical_url: CanonicalUrl(candidate.result.canonical_url.0.clone()),
                final_url,
                content_hash,
                fetch_method,
                extractor_used,
                content_type: fetched.content_type,
                size: document_size,
                retrieved_at: fetched.retrieved_at,
                status,
                content,
                title,
                author,
                published_at,
                metadata,
            };
            if depth < self.max_depth {
                for link in discover_links(&fetched.body, &document.final_url) {
                    if visited.insert(link.as_str().to_owned()) {
                        let child = child_result(&candidate.result, link);
                        original_ranks.insert(search_result_id(&child), child.rank);
                        queue.push_back((
                            DeepCandidate {
                                result: child,
                                storage_rights: candidate.storage_rights,
                            },
                            depth + 1,
                        ));
                    }
                }
            }
            if let Some(cache) = &self.cache {
                if !cache
                    .put(
                        &document,
                        self.extractor.version(),
                        candidate.storage_rights,
                        None,
                        None,
                    )
                    .await
                    && candidate.storage_rights
                {
                    response.degradations.push(degradation("document_cache_write_failed", "Document cache write was skipped or failed; current Document is unaffected"));
                }
            }
            response.documents.push(document);
        }
        (response.evidence, response.evidence_v2) =
            crate::evidence::analyze_evidence_bundle(&request.query, &response.documents);
        if let Some(engine) = &self.ranking_v2 {
            match engine
                .rank(
                    &request.query,
                    &response.documents,
                    &response.evidence,
                    &original_ranks,
                )
                .await
            {
                Ok(ranking) => response.ranking_v2 = ranking,
                Err(_) => {
                    response.ranking_v2 = rejected_output();
                    response.degradations.push(degradation(
                        "ranking_v2_failed",
                        "Ranking v2 failed; original Search ranking remains authoritative",
                    ));
                }
            }
        }
        if let Some(analyzer) = &self.gap_analyzer {
            let analysis = analyzer.analyze(
                &request.query,
                &request.search_plan,
                &response.documents,
                &response.evidence,
            );
            let proposals = analyzer.proposals(&analysis);
            response.gaps = analysis.gaps;
            for proposal in proposals {
                let raw_query = proposal.recommended_query.clone().unwrap_or_default();
                let estimated_cost = proposal.estimated_cost.unwrap_or(1);
                let expected_gain = proposal.expected_gain.unwrap_or(1);
                let mut subquery = SubQuery {
                    schema_version: SCHEMA_VERSION.into(),
                    raw_query: raw_query.clone(),
                    reason: proposal.reason.clone(),
                    gap_type: proposal.gap_type,
                    estimated_cost,
                    expected_gain,
                    actual_gain: 0,
                    status: SubQueryStatus::Proposed,
                    results: vec![],
                    errors: vec![],
                };
                let parsed = match crate::query::parse_query(raw_query) {
                    Ok(query) => query,
                    Err(_) => {
                        subquery.status = SubQueryStatus::Invalid;
                        response.subqueries.push(subquery);
                        continue;
                    }
                };
                let Some(executor) = &self.subquery_executor else {
                    response.subqueries.push(subquery);
                    continue;
                };
                if tokio::time::Instant::now() >= deadline {
                    subquery.status = SubQueryStatus::RejectedBudget;
                    response.degradations.push(degradation(
                        "time_exhausted",
                        "SubQuery was rejected because the Deep deadline expired",
                    ));
                    response.subqueries.push(subquery);
                    continue;
                }
                if let Err(cause) = self.budget.reserve_subquery(estimated_cost) {
                    subquery.status = SubQueryStatus::RejectedBudget;
                    response.degradations.push(degradation(
                        &cause.to_string(),
                        "SubQuery was rejected by Deep Budget",
                    ));
                    response.subqueries.push(subquery);
                    continue;
                }
                match tokio::time::timeout_at(deadline, executor.execute(parsed)).await {
                    Ok(Ok(expansion)) => {
                        subquery.errors = expansion.errors;
                        if expansion.status == SearchStatus::Failure {
                            subquery.status = SubQueryStatus::Failed;
                        } else {
                            subquery.results = expansion
                                .results
                                .into_iter()
                                .filter(|result| {
                                    known_urls.insert(result.canonical_url.0.as_str().to_owned())
                                })
                                .collect();
                            subquery.actual_gain = subquery.results.len() as u32;
                            subquery.status = SubQueryStatus::Executed;
                        }
                    }
                    Ok(Err(_)) => {
                        subquery.status = SubQueryStatus::Failed;
                        subquery.errors.push(CompositeError {
                            code: "subquery_failed".into(),
                            message: "SubQuery execution failed".into(),
                            providers: vec![],
                            recoverable: true,
                        });
                    }
                    Err(_) => {
                        subquery.status = SubQueryStatus::Failed;
                        subquery.errors.push(CompositeError {
                            code: "subquery_timeout".into(),
                            message: "SubQuery execution exceeded the Deep deadline".into(),
                            providers: vec![],
                            recoverable: true,
                        });
                    }
                }
                response.subqueries.push(subquery);
            }
        }
        response.elapsed_ms = started.elapsed().as_millis() as u64;
        response
    }

    pub fn budget_snapshot(&self) -> crate::budget::DeepBudgetSnapshot {
        self.budget.snapshot()
    }
}

fn default_headers() -> BTreeMap<String, String> {
    BTreeMap::from([
        (
            "accept".into(),
            "text/html,application/xhtml+xml,text/plain;q=0.9".into(),
        ),
        ("user-agent".into(), "AMATL/0.1 (+safe-deep-fetch)".into()),
    ])
}

fn hex_digest(value: &[u8]) -> String {
    format!("{:x}", Sha256::digest(value))
}
fn search_result_id(result: &SearchResult) -> String {
    hex_digest(result.canonical_url.0.as_str().as_bytes())
}
fn degradation(code: &str, message: &str) -> Degradation {
    Degradation {
        code: code.into(),
        component: "deep".into(),
        message: message.into(),
    }
}
fn fetch_code(error: &FetchError) -> &'static str {
    match error {
        FetchError::EgressDenied => "egress_denied",
        FetchError::BlockedUrl(_) | FetchError::AddressBlocked => "fetch_blocked",
        FetchError::ByteLimit => "byte_limit",
        FetchError::RedirectLimit => "redirect_limit",
        FetchError::Timeout => "fetch_timeout",
        _ => "fetch_failed",
    }
}
fn extract_code(error: &ExtractError) -> &'static str {
    match error {
        ExtractError::Unavailable => "extractor_unavailable",
        ExtractError::Timeout => "extractor_timeout",
        _ => "extractor_failed",
    }
}
fn fetch_error(result: &SearchResult, error: &FetchError) -> CompositeError {
    CompositeError {
        code: fetch_code(error).into(),
        message: format!("Deep fetch failed for {}", result.domain),
        providers: result.providers.clone(),
        recoverable: true,
    }
}

fn discover_links(body: &[u8], base: &url::Url) -> Vec<url::Url> {
    let Ok(html) = std::str::from_utf8(body) else {
        return vec![];
    };
    let Ok(selector) = scraper::Selector::parse("a[href]") else {
        return vec![];
    };
    let document = scraper::Html::parse_document(html);
    let base_host = base.host_str();
    document
        .select(&selector)
        .filter_map(|element| element.value().attr("href"))
        .filter_map(|href| base.join(href).ok())
        .filter(|url| matches!(url.scheme(), "http" | "https") && url.host_str() == base_host)
        .take(32)
        .collect()
}

fn child_result(parent: &SearchResult, mut url: url::Url) -> SearchResult {
    url.set_fragment(None);
    SearchResult {
        schema_version: SCHEMA_VERSION.into(),
        rank: parent.rank,
        title: None,
        original_url: OriginalUrl(url.clone()),
        canonical_url: CanonicalUrl(url.clone()),
        domain: url.host_str().unwrap_or_default().to_owned(),
        snippet: None,
        providers: parent.providers.clone(),
        published_at: None,
        status: parent.status.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crawl_discovers_only_same_origin_http_links() {
        let base = url::Url::parse("https://example.com/root").unwrap();
        let links = discover_links(b"<a href='/a'>a</a><a href='https://other.test/x'>x</a><a href='file:///etc/passwd'>f</a>", &base);
        assert_eq!(
            links,
            vec![url::Url::parse("https://example.com/a").unwrap()]
        );
    }
}
