use amatl_core::{
    classify, parse_query, Budget, CanonicalUrl, DeepBudget, DeepCandidate, DeepOrchestrator,
    DeepRequest, DocumentCache, DocumentCachePolicy, DocumentStatus, ExtractError,
    ExtractionResult, Extractor, FetchError, FetchRequest, FetchResult, Fetcher, GapAnalyzer,
    GapPolicyV1, OriginalUrl, Rank, RankingV2Engine, RankingV2Policy, RankingV2Status, RenderError,
    RenderResult, Renderer, RendererPool, ResultStatus, SearchResponse, SearchResult, SearchStatus,
    SqliteStorage, SubQueryExecutionError, SubQueryExecutor, SubQueryStatus, SCHEMA_VERSION,
};
use async_trait::async_trait;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use url::Url;

struct FixedFetcher(Result<FetchResult, FetchError>);

#[async_trait]
impl Fetcher for FixedFetcher {
    async fn fetch(&self, _: FetchRequest) -> Result<FetchResult, FetchError> {
        self.0.clone()
    }
}

struct FixedExtractor {
    version: &'static str,
    result: Result<ExtractionResult, ExtractError>,
}

#[async_trait]
impl Extractor for FixedExtractor {
    fn name(&self) -> &str {
        "fixed"
    }
    fn version(&self) -> &str {
        self.version
    }
    async fn extract(&self, _: &[u8]) -> Result<ExtractionResult, ExtractError> {
        self.result.clone()
    }
}

struct NoRenderer;

struct FixedSubQueryExecutor {
    calls: Arc<AtomicUsize>,
}

#[async_trait]
impl SubQueryExecutor for FixedSubQueryExecutor {
    async fn execute(
        &self,
        query: amatl_core::Query,
    ) -> Result<SearchResponse, SubQueryExecutionError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let url = Url::parse("https://independent.example/result").unwrap();
        Ok(SearchResponse {
            schema_version: SCHEMA_VERSION.into(),
            query: query.raw_query,
            status: SearchStatus::Success,
            results: vec![SearchResult {
                schema_version: SCHEMA_VERSION.into(),
                rank: Rank::new(1).unwrap(),
                title: Some("Independent source".into()),
                original_url: OriginalUrl(url.clone()),
                canonical_url: CanonicalUrl(url),
                domain: "independent.example".into(),
                snippet: None,
                providers: vec!["mock-expansion".into()],
                published_at: None,
                status: ResultStatus::Visible,
            }],
            providers_used: vec!["mock-expansion".into()],
            providers_failed: vec![],
            providers_partial: vec![],
            errors: vec![],
            degradations: vec![],
            elapsed_ms: 1,
            total_results: None,
            page: None,
            page_size: None,
        })
    }
}

#[async_trait]
impl Renderer for NoRenderer {
    fn available(&self) -> bool {
        false
    }
    async fn render(&self, _: &Url) -> Result<RenderResult, RenderError> {
        Err(RenderError::Unavailable)
    }
}

fn result() -> SearchResult {
    SearchResult {
        schema_version: SCHEMA_VERSION.into(),
        rank: Rank::new(1).unwrap(),
        title: Some("Example".into()),
        original_url: OriginalUrl(Url::parse("https://example.com/article?utm_source=x").unwrap()),
        canonical_url: CanonicalUrl(Url::parse("https://example.com/article").unwrap()),
        domain: "example.com".into(),
        snippet: None,
        providers: vec!["mock".into()],
        published_at: None,
        status: ResultStatus::Visible,
    }
}

fn fetch_ok() -> FetchResult {
    FetchResult {
        final_url: amatl_core::FinalUrl(Url::parse("https://example.com/final").unwrap()),
        status: 200,
        headers_safe: BTreeMap::new(),
        content_type: Some("text/html".into()),
        body: b"<html><p>body</p></html>".to_vec(),
        size: 24,
        redirect_chain: vec![],
        retrieved_at: "2026-08-12T00:00:00Z".into(),
    }
}

fn extraction_ok() -> ExtractionResult {
    ExtractionResult {
        content: "body".into(),
        format: "text".into(),
        title: Some("Example".into()),
        author: None,
        published_at: None,
        metadata: BTreeMap::new(),
        extractor_used: "fixed-v1".into(),
        status: "success".into(),
    }
}

fn orchestrator(
    fetch: Result<FetchResult, FetchError>,
    extraction: Result<ExtractionResult, ExtractError>,
) -> DeepOrchestrator {
    DeepOrchestrator::new(
        DeepBudget::new(2, 1024, 2, 1, 2, 1000).with_gap_limits(2, 2),
        Arc::new(FixedFetcher(fetch)),
        Arc::new(FixedExtractor {
            version: "fixed-v1",
            result: extraction,
        }),
        RendererPool::new(Arc::new(NoRenderer), 1),
        None,
        1000,
        1024,
        2,
        2,
        1,
    )
}

fn request(candidates: Vec<DeepCandidate>) -> DeepRequest {
    request_for("query", candidates)
}

fn request_for(raw_query: &str, candidates: Vec<DeepCandidate>) -> DeepRequest {
    let query = parse_query(raw_query.into()).unwrap();
    let plan = amatl_core::planning::build_search_plan(
        query.clone(),
        classify(&query),
        amatl_core::router::RoutingRecommendation {
            selected_providers: vec![],
            provider_budget_requests: BTreeMap::new(),
            debug_reasons: vec![],
        },
        &mut Budget::new(1, 1_000),
    );
    DeepRequest {
        query,
        search_plan: plan,
        candidates,
    }
}

#[tokio::test]
async fn enriches_search_result_without_changing_search_contract() {
    let search = result();
    let mut deep = orchestrator(Ok(fetch_ok()), Ok(extraction_ok()));
    let output = deep
        .enrich(request(vec![DeepCandidate {
            result: search.clone(),
            storage_rights: false,
        }]))
        .await;
    assert_eq!(output.documents.len(), 1);
    assert_eq!(output.documents[0].status, DocumentStatus::Enriched);
    assert_eq!(
        output.documents[0].final_url.as_str(),
        "https://example.com/final"
    );
    assert_eq!(
        search.canonical_url.0.as_str(),
        "https://example.com/article"
    );
    assert_eq!(output.evidence.len(), 1);
    assert_eq!(output.evidence_v2.len(), 1);
    assert_eq!(output.evidence_v2[0].evidence_version, "v2");
    assert_eq!(
        output.evidence_v2[0].evidence_score,
        output.evidence[0].evidence_score
    );
    assert!(!output.evidence_v2[0].fragments.is_empty());
    assert_eq!(output.ranking_v2.status, RankingV2Status::Disabled);
    assert!(output.gaps.is_empty() && output.subqueries.is_empty());
}

#[tokio::test]
async fn ranking_v2_runs_only_inside_deep_after_benchmark_gate() {
    let mut deep = orchestrator(Ok(fetch_ok()), Ok(extraction_ok()))
        .with_ranking_v2(RankingV2Engine::new(RankingV2Policy::default()).unwrap());
    let output = deep
        .enrich(request(vec![DeepCandidate {
            result: result(),
            storage_rights: false,
        }]))
        .await;
    assert_eq!(output.ranking_v2.status, RankingV2Status::Applied);
    assert_eq!(output.ranking_v2.results.len(), 1);
    assert_eq!(
        output.ranking_v2.results[0].evidence_score,
        output.evidence[0].evidence_score
    );
    assert!(output.gaps.is_empty() && output.subqueries.is_empty());
}

#[tokio::test]
async fn gap_expansion_is_executed_once_by_deep_and_reports_actual_gain() {
    let calls = Arc::new(AtomicUsize::new(0));
    let mut deep = orchestrator(Ok(fetch_ok()), Ok(extraction_ok()))
        .with_gap_analyzer(GapAnalyzer::new(GapPolicyV1::default()).unwrap())
        .with_subquery_executor(Arc::new(FixedSubQueryExecutor {
            calls: calls.clone(),
        }));
    let output = deep
        .enrich(request(vec![DeepCandidate {
            result: result(),
            storage_rights: false,
        }]))
        .await;
    assert!(!output.gaps.is_empty());
    assert_eq!(output.subqueries.len(), 1);
    assert_eq!(output.subqueries[0].status, SubQueryStatus::Executed);
    assert_eq!(output.subqueries[0].actual_gain, 1);
    assert_eq!(output.subqueries[0].results.len(), 1);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(deep.budget_snapshot().remaining_subqueries, 1);
}

#[tokio::test]
async fn deep_never_executes_a_third_gap_proposal() {
    let calls = Arc::new(AtomicUsize::new(0));
    let mut deep = orchestrator(Ok(fetch_ok()), Ok(extraction_ok()))
        .with_gap_analyzer(GapAnalyzer::new(GapPolicyV1::default()).unwrap())
        .with_subquery_executor(Arc::new(FixedSubQueryExecutor {
            calls: calls.clone(),
        }));
    let output = deep
        .enrich(request_for(
            "official RFC filetype:pdf",
            vec![DeepCandidate {
                result: result(),
                storage_rights: false,
            }],
        ))
        .await;
    assert!(output.gaps.len() >= 3);
    assert_eq!(output.subqueries.len(), 2);
    assert!(output
        .subqueries
        .iter()
        .all(|subquery| subquery.status == SubQueryStatus::Executed));
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert_eq!(deep.budget_snapshot().remaining_subqueries, 0);
}

#[tokio::test]
async fn missing_extractor_keeps_a_superficial_document() {
    let mut deep = orchestrator(Ok(fetch_ok()), Err(ExtractError::Unavailable));
    let output = deep
        .enrich(request(vec![DeepCandidate {
            result: result(),
            storage_rights: false,
        }]))
        .await;
    assert_eq!(output.documents[0].status, DocumentStatus::Superficial);
    assert!(output.documents[0].content.is_none());
    assert!(output
        .degradations
        .iter()
        .any(|value| value.code == "extractor_unavailable"));
}

#[tokio::test]
async fn fetch_failure_is_partial_and_never_invalidates_input_search_result() {
    let search = result();
    let mut deep = orchestrator(
        Err(FetchError::AddressBlocked),
        Err(ExtractError::Unavailable),
    );
    let output = deep
        .enrich(request(vec![DeepCandidate {
            result: search.clone(),
            storage_rights: false,
        }]))
        .await;
    assert!(output.documents.is_empty());
    assert_eq!(output.errors[0].code, "fetch_blocked");
    assert_eq!(search.title.as_deref(), Some("Example"));
}

#[tokio::test]
async fn document_cache_is_rights_gated_versioned_and_drops_body_by_default() {
    let path = std::env::temp_dir().join(format!(
        "amatl-deep-cache-{}-{}.sqlite3",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let storage = SqliteStorage::open(&path).await.unwrap();
    let cache = DocumentCache::new(
        storage,
        DocumentCachePolicy {
            enabled: true,
            ttl_seconds: 100,
            max_entries: 10,
            max_bytes: 10_000,
            store_content: false,
            stale_while_revalidate_seconds: 0,
        },
    );
    let mut deep = DeepOrchestrator::new(
        DeepBudget::new(1, 1024, 1, 1, 1, 1000),
        Arc::new(FixedFetcher(Ok(fetch_ok()))),
        Arc::new(FixedExtractor {
            version: "fixed-v1",
            result: Ok(extraction_ok()),
        }),
        RendererPool::new(Arc::new(NoRenderer), 1),
        Some(cache.clone()),
        1000,
        1024,
        1,
        1,
        1,
    );
    let output = deep
        .enrich(request(vec![DeepCandidate {
            result: result(),
            storage_rights: true,
        }]))
        .await;
    let document = &output.documents[0];
    let cached = cache
        .get(
            document.canonical_url.0.as_str(),
            &document.content_hash,
            "fixed-v1",
        )
        .await
        .unwrap();
    assert!(cached.content.is_none());
    assert!(cache
        .get(
            document.canonical_url.0.as_str(),
            &document.content_hash,
            "fixed-v2"
        )
        .await
        .is_none());
    assert_eq!(cache.stats().await.entries, 1);
    assert_eq!(cache.purge().await, 1);
    drop(cache);
    let _ = std::fs::remove_file(path);
}
