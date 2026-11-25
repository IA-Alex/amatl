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
            complementarity: None,
        })
    }
}

#[async_trait]
impl Renderer for NoRenderer {
    fn available(&self) -> bool {
        false
    }
    async fn render(&self, _: &[u8]) -> Result<RenderResult, RenderError> {
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
async fn shared_deep_budget_stops_selected_candidates_and_keeps_provenance() {
    let mut first = result();
    first.providers = vec!["selected-provider".into()];
    let mut second = result();
    second.canonical_url = CanonicalUrl(Url::parse("https://example.com/second").unwrap());
    second.original_url = OriginalUrl(Url::parse("https://example.com/second").unwrap());
    second.rank = Rank::new(2).unwrap();
    let mut deep = DeepOrchestrator::new(
        DeepBudget::new(1, 1024, 2, 1, 2, 1000),
        Arc::new(FixedFetcher(Ok(fetch_ok()))),
        Arc::new(FixedExtractor {
            version: "fixed-v1",
            result: Ok(extraction_ok()),
        }),
        RendererPool::new(Arc::new(NoRenderer), 1),
        None,
        1000,
        1024,
        2,
        2,
        0,
    );
    let output = deep
        .enrich(request(vec![
            DeepCandidate {
                result: first,
                storage_rights: false,
            },
            DeepCandidate {
                result: second,
                storage_rights: false,
            },
        ]))
        .await;
    assert_eq!(output.documents.len(), 1);
    assert_eq!(output.evidence.len(), 1);
    assert_eq!(
        output.evidence[0].document_id,
        output.documents[0].search_result_id
    );
    assert_eq!(
        output.documents[0].metadata.get("search_providers"),
        Some(&"selected-provider".into())
    );
    assert!(output
        .degradations
        .iter()
        .any(|degradation| degradation.code == "fetch_limit"));
    assert_eq!(deep.budget_snapshot().remaining_fetches, 0);
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
    let storage = SqliteStorage::open(&path, amatl_core::config::SqliteLockingMode::Normal)
        .await
        .unwrap();
    let cache = DocumentCache::new(
        storage,
        DocumentCachePolicy {
            enabled: true,
            ttl_seconds: 100,
            max_entries: 10,
            max_bytes: 10_000,
            store_content: false,
            stale_while_revalidate_seconds: 0,
            model_version: None,
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

/// Serves a distinct body per path so a crawl can be observed end to end.
struct RoutingFetcher {
    robots: (u16, &'static str),
    seen: Arc<std::sync::Mutex<Vec<String>>>,
}

#[async_trait]
impl Fetcher for RoutingFetcher {
    async fn fetch(&self, request: FetchRequest) -> Result<FetchResult, FetchError> {
        let url = request.url.clone();
        self.seen.lock().unwrap().push(url.to_string());
        let (status, body) = if url.path() == "/robots.txt" {
            (self.robots.0, self.robots.1.to_owned())
        } else if url.path() == "/article" {
            (200, "<html><a href='/deeper'>deeper</a></html>".to_owned())
        } else {
            (200, "<html><p>discovered</p></html>".to_owned())
        };
        Ok(FetchResult {
            final_url: amatl_core::FinalUrl(url),
            status,
            headers_safe: BTreeMap::new(),
            content_type: Some("text/html".into()),
            size: body.len() as u64,
            body: body.into_bytes(),
            redirect_chain: vec![],
            retrieved_at: "2026-08-14T00:00:00Z".into(),
        })
    }
}

fn crawling_orchestrator(fetcher: Arc<dyn Fetcher>) -> DeepOrchestrator {
    DeepOrchestrator::new(
        DeepBudget::new(4, 100_000, 2, 1, 4, 5_000).with_gap_limits(0, 0),
        fetcher,
        Arc::new(FixedExtractor {
            version: "fixed-v1",
            result: Ok(extraction_ok()),
        }),
        RendererPool::new(Arc::new(NoRenderer), 1),
        None,
        5_000,
        100_000,
        2,
        2,
        // One level of crawl, which is the default configuration.
        1,
    )
}

#[tokio::test]
async fn a_disallowed_discovered_link_is_not_crawled() {
    let seen = Arc::new(std::sync::Mutex::new(vec![]));
    let fetcher = Arc::new(RoutingFetcher {
        robots: (200, "User-agent: *\nDisallow: /deeper\n"),
        seen: seen.clone(),
    });
    let mut deep = crawling_orchestrator(fetcher).with_robots(amatl_core::RobotsCache::new(
        Arc::new(RoutingFetcher {
            robots: (200, "User-agent: *\nDisallow: /deeper\n"),
            seen: seen.clone(),
        }),
        1_000,
        64 * 1024,
    ));
    let output = deep
        .enrich(request(vec![DeepCandidate {
            result: result(),
            storage_rights: false,
        }]))
        .await;

    // The URL the user asked for is fetched; the link AMATL discovered is not.
    let seen = seen.lock().unwrap().clone();
    assert!(seen.iter().any(|url| url.ends_with("/article")), "{seen:?}");
    assert!(!seen.iter().any(|url| url.ends_with("/deeper")), "{seen:?}");
    assert!(
        seen.iter().any(|url| url.ends_with("/robots.txt")),
        "{seen:?}"
    );
    assert_eq!(output.documents.len(), 1);
    assert!(
        output
            .degradations
            .iter()
            .any(|degradation| degradation.code == "robots_disallowed"),
        "{:?}",
        output.degradations
    );
}

#[tokio::test]
async fn an_allowed_discovered_link_is_still_crawled() {
    let seen = Arc::new(std::sync::Mutex::new(vec![]));
    let mut deep = crawling_orchestrator(Arc::new(RoutingFetcher {
        robots: (404, ""),
        seen: seen.clone(),
    }))
    .with_robots(amatl_core::RobotsCache::new(
        Arc::new(RoutingFetcher {
            // No robots.txt published: RFC 9309 allows the crawl.
            robots: (404, ""),
            seen: seen.clone(),
        }),
        1_000,
        64 * 1024,
    ));
    let output = deep
        .enrich(request(vec![DeepCandidate {
            result: result(),
            storage_rights: false,
        }]))
        .await;
    let seen = seen.lock().unwrap().clone();
    assert!(seen.iter().any(|url| url.ends_with("/deeper")), "{seen:?}");
    assert_eq!(output.documents.len(), 2);
}

#[tokio::test]
async fn an_unreachable_robots_file_stops_the_crawl() {
    struct DeadRobots;

    #[async_trait]
    impl Fetcher for DeadRobots {
        async fn fetch(&self, _: FetchRequest) -> Result<FetchResult, FetchError> {
            Err(FetchError::Timeout)
        }
    }

    let seen = Arc::new(std::sync::Mutex::new(vec![]));
    let mut deep = crawling_orchestrator(Arc::new(RoutingFetcher {
        robots: (200, ""),
        seen: seen.clone(),
    }))
    .with_robots(amatl_core::RobotsCache::new(
        Arc::new(DeadRobots),
        200,
        1024,
    ));
    let output = deep
        .enrich(request(vec![DeepCandidate {
            result: result(),
            storage_rights: false,
        }]))
        .await;
    let seen = seen.lock().unwrap().clone();
    assert!(!seen.iter().any(|url| url.ends_with("/deeper")), "{seen:?}");
    assert!(
        output
            .degradations
            .iter()
            .any(|degradation| degradation.code == "robots_unavailable"),
        "{:?}",
        output.degradations
    );
}

/// Absolute path to the isolation harness, resolved from the workspace root.
///
/// Tests run with the crate directory as CWD, so a repository-relative path
/// would not resolve.
#[cfg(test)]
fn harness_path() -> String {
    std::env::var("AMATL_CHROMIUM_SANDBOX").unwrap_or_else(|_| {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../packaging/amatl-chromium-sandbox")
            .to_string_lossy()
            .into_owned()
    })
}

/// Drives the real isolation harness through `ChromiumRenderer`.
///
/// Gated like the Trafilatura integration test: inert unless
/// `AMATL_CHROMIUM_INTEGRATION=1`, because it needs `bwrap`, `systemd-run`, a
/// Chromium binary and unprivileged user namespaces.
#[tokio::test]
async fn chromium_renderer_executes_scripts_through_the_isolation_harness() {
    if std::env::var("AMATL_CHROMIUM_INTEGRATION").as_deref() != Ok("1") {
        return;
    }
    use amatl_core::{ChromiumRenderer, RendererConfig};

    let harness = harness_path();
    let config = RendererConfig {
        enabled: true,
        sandbox_path: std::fs::canonicalize(&harness)
            .expect("harness path")
            .to_string_lossy()
            .into_owned(),
        timeout_ms: 30_000,
        ..RendererConfig::default()
    };
    let renderer = ChromiumRenderer::detect(&config);
    assert!(
        renderer.available(),
        "harness prerequisites missing: {}",
        renderer.unavailable_reason()
    );

    let html = br#"<!doctype html><html><body>
        <div id="target">before</div>
        <script>document.getElementById('target').textContent = 'RENDERED_BY_JS';</script>
        </body></html>"#;
    let rendered = renderer.render(html).await.expect("render succeeds");
    let dom = String::from_utf8_lossy(&rendered.dom);
    assert!(
        dom.contains("RENDERED_BY_JS"),
        "scripts did not run; DOM was: {dom}"
    );
    assert!(!dom.contains(">before<"), "the pre-script DOM was returned");
}

/// The renderer's environment handling, proven on the production code path.
///
/// `scrub_environment` is unit-tested, but the finding was about *wiring*: a
/// spawn site that simply forgets to call it. This drives the real
/// `ChromiumRenderer::render` with a stand-in harness that dumps the
/// environment it was given into the DOM it returns, so both halves of the
/// contract are checked where they matter: the service's credentials are gone,
/// and the variables the harness legitimately needs (a `PATH` for its own
/// tools, the `AMATL_CHROMIUM_*` limits) are still there.
#[cfg(unix)]
#[tokio::test]
async fn the_renderer_hands_the_harness_a_scrubbed_environment() {
    if std::env::var("AMATL_CHROMIUM_INTEGRATION").as_deref() != Ok("1") {
        return;
    }
    use amatl_core::{ChromiumRenderer, RendererConfig};
    use std::os::unix::fs::PermissionsExt;

    const CANARY: &str = "AMATL_RENDER_ENV_CANARY_4d19";
    let directory = std::env::temp_dir().join(format!("amatl-fake-harness-{}", std::process::id()));
    std::fs::create_dir_all(&directory).expect("scratch directory");
    let harness = directory.join("amatl-chromium-sandbox");
    std::fs::write(
        &harness,
        b"#!/bin/sh\ncat \"$1\" >/dev/null\nenv > \"$2\"\n",
    )
    .expect("harness");
    std::fs::set_permissions(&harness, std::fs::Permissions::from_mode(0o700)).unwrap();

    std::env::set_var(CANARY, "server-token-canary");
    let config = RendererConfig {
        enabled: true,
        sandbox_path: harness.to_string_lossy().into_owned(),
        timeout_ms: 5_000,
        ..RendererConfig::default()
    };
    let renderer = ChromiumRenderer::detect(&config);
    let rendered = renderer.render(b"<p>x</p>").await;
    std::env::remove_var(CANARY);
    let _ = std::fs::remove_dir_all(&directory);

    assert!(
        renderer.available(),
        "harness prerequisites missing: {}",
        renderer.unavailable_reason()
    );
    let dom =
        String::from_utf8_lossy(&rendered.expect("the stand-in harness succeeds").dom).into_owned();
    assert!(
        !dom.contains(CANARY),
        "the harness inherited a credential: {dom}"
    );
    assert!(
        dom.lines().any(|line| line.starts_with("PATH=")),
        "the harness lost PATH and cannot run its own tools: {dom}"
    );
    assert!(
        dom.lines()
            .any(|line| line.starts_with("AMATL_CHROMIUM_MAX_DOM_BYTES=")),
        "the harness lost the limits the renderer passes it: {dom}"
    );
}

/// The renderer must not be able to reach the network, including loopback.
///
/// This is the invariant the whole design rests on: `SafeFetcher` stays the
/// only owner of egress. A page that tries to fetch must fail inside the
/// sandbox rather than reaching a local listener.
#[tokio::test]
async fn rendered_pages_cannot_reach_the_network() {
    if std::env::var("AMATL_CHROMIUM_INTEGRATION").as_deref() != Ok("1") {
        return;
    }
    use amatl_core::{ChromiumRenderer, RendererConfig};

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let reached = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = reached.clone();
    tokio::spawn(async move {
        while let Ok((_stream, _)) = listener.accept().await {
            flag.store(true, std::sync::atomic::Ordering::SeqCst);
        }
    });

    let harness = harness_path();
    let config = RendererConfig {
        enabled: true,
        sandbox_path: std::fs::canonicalize(&harness)
            .expect("harness path")
            .to_string_lossy()
            .into_owned(),
        timeout_ms: 30_000,
        ..RendererConfig::default()
    };
    let renderer = ChromiumRenderer::detect(&config);
    assert!(renderer.available(), "{}", renderer.unavailable_reason());

    // The outcome markers are assembled at runtime so the literals never
    // appear in the script source: the dumped DOM includes the script text, so
    // searching for a literal would match whether or not it ever executed.
    let html = format!(
        r#"<!doctype html><html><body><div id="r">pending</div><script>
        var mark = function (suffix) {{
          document.getElementById('r').textContent = 'NET' + '_' + suffix;
        }};
        fetch("http://127.0.0.1:{port}/")
          .then(function () {{ mark('OK'); }})
          .catch(function () {{ mark('FAIL'); }});
        </script></body></html>"#
    );
    let rendered = renderer.render(html.as_bytes()).await.expect("render");
    let dom = String::from_utf8_lossy(&rendered.dom);

    // The authoritative signal: no connection may ever arrive, regardless of
    // whether the page's promise had settled by the time the DOM was dumped.
    assert!(
        !reached.load(std::sync::atomic::Ordering::SeqCst),
        "a connection arrived at the loopback listener from inside the sandbox"
    );
    assert!(
        !dom.contains("NET_OK"),
        "the page observed a successful fetch to loopback: {dom}"
    );
    // Guards against the test passing vacuously: the fetch must actually have
    // run and been refused, not merely still be pending when the DOM was
    // dumped.
    assert!(
        dom.contains("NET_FAIL"),
        "the fetch never resolved, so isolation was not exercised: {dom}"
    );
}

/// A fetcher that records the fragmentless path of every URL it is asked for,
/// so a test can count how many times one HTTP resource is actually fetched.
struct FragmentRecordingFetcher {
    seen: Arc<std::sync::Mutex<Vec<String>>>,
}

#[async_trait]
impl Fetcher for FragmentRecordingFetcher {
    async fn fetch(&self, request: FetchRequest) -> Result<FetchResult, FetchError> {
        let url = request.url.clone();
        // Record the resource identity: scheme + host + path + query, with the
        // fragment dropped. That is what "fetching this resource" means.
        let mut identity = url.clone();
        identity.set_fragment(None);
        self.seen.lock().unwrap().push(identity.to_string());
        let body = if url.path() == "/article" {
            // Three fragments of the *same* resource. A correct crawl treats
            // them as one target and fetches `/article` once.
            "<html><a href='/article#a'>a</a><a href='/article#b'>b</a><a href='/article#c'>c</a></html>"
                .to_owned()
        } else {
            "<html><p>discovered</p></html>".to_owned()
        };
        Ok(FetchResult {
            final_url: amatl_core::FinalUrl(url),
            status: 200,
            headers_safe: BTreeMap::new(),
            content_type: Some("text/html".into()),
            size: body.len() as u64,
            body: body.into_bytes(),
            redirect_chain: vec![],
            retrieved_at: "2026-08-14T00:00:00Z".into(),
        })
    }
}

#[tokio::test]
async fn fragments_of_one_resource_are_not_crawled_as_separate_entries() {
    // Regression: `discover_links` returned URLs *with* their fragment, and the
    // `visited` set was keyed on that fragment-bearing string, while
    // `child_result` stripped the fragment. Three anchors `#a`/`#b`/`#c` on one
    // page therefore became three distinct `visited` keys and three queued
    // children that all resolved to the same fragmentless URL — so the origin
    // was fetched three times and three fetch/crawl/byte budget units were
    // spent on one resource. Deduplication now keys on the resource identity.
    let seen = Arc::new(std::sync::Mutex::new(vec![]));
    let mut deep = crawling_orchestrator(Arc::new(FragmentRecordingFetcher { seen: seen.clone() }))
        .with_robots(amatl_core::RobotsCache::new(
            Arc::new(FragmentRecordingFetcher {
                seen: Arc::new(std::sync::Mutex::new(vec![])),
            }),
            1_000,
            64 * 1024,
        ));
    let output = deep
        .enrich(request(vec![DeepCandidate {
            result: result(),
            storage_rights: false,
        }]))
        .await;

    let seen = seen.lock().unwrap().clone();
    let article_fetches = seen.iter().filter(|url| url.ends_with("/article")).count();
    assert_eq!(
        article_fetches, 1,
        "one resource with three fragment links must be fetched once: {seen:?}"
    );
    // The three fragment links all name the seed resource, which is already
    // visited, so the crawl adds no second Document: exactly one resource was
    // processed, not one per fragment.
    assert_eq!(output.documents.len(), 1, "{:?}", output.degradations);
}

/// A fetcher that records the `timeout_ms` it was handed for the deepest link.
struct TimeoutRecordingFetcher {
    seen: Arc<std::sync::Mutex<Vec<(String, u64)>>>,
}

#[async_trait]
impl Fetcher for TimeoutRecordingFetcher {
    async fn fetch(&self, request: FetchRequest) -> Result<FetchResult, FetchError> {
        let url = request.url.clone();
        self.seen
            .lock()
            .unwrap()
            .push((url.to_string(), request.timeout_ms));
        let body = if url.path() == "/robots.txt" {
            // A long, but still accepted, crawl delay: the next discovered link
            // must wait for it, shrinking the time left before the deadline.
            "User-agent: *\nCrawl-delay: 1\n".to_owned()
        } else if url.path() == "/article" {
            "<html><a href='/deeper'>deeper</a></html>".to_owned()
        } else {
            "<html><p>discovered</p></html>".to_owned()
        };
        Ok(FetchResult {
            final_url: amatl_core::FinalUrl(url),
            status: 200,
            headers_safe: BTreeMap::new(),
            content_type: Some("text/html".into()),
            size: body.len() as u64,
            body: body.into_bytes(),
            redirect_chain: vec![],
            retrieved_at: "2026-08-14T00:00:00Z".into(),
        })
    }
}

#[tokio::test]
async fn fetch_timeout_reflects_the_time_left_after_a_crawl_delay() {
    // Regression: `fetch_timeout` was computed from the `now` sampled at the
    // *top* of the loop iteration, before the robots `crawl_delay` sleep. After
    // the sleep the deadline was closer than that stale `now` implied, so the
    // fetch was granted more timeout than actually remained and the network
    // request could outlive the global Deep deadline. The timeout is now
    // recomputed from the current instant.
    let seen = Arc::new(std::sync::Mutex::new(vec![]));
    let mut deep = crawling_orchestrator(Arc::new(TimeoutRecordingFetcher { seen: seen.clone() }))
        .with_robots(amatl_core::RobotsCache::new(
            Arc::new(TimeoutRecordingFetcher {
                seen: Arc::new(std::sync::Mutex::new(vec![])),
            }),
            1_000,
            64 * 1024,
        ));
    let _ = deep
        .enrich(request(vec![DeepCandidate {
            result: result(),
            storage_rights: false,
        }]))
        .await;

    let seen = seen.lock().unwrap().clone();
    let seed_timeout = seen
        .iter()
        .find(|(url, _)| url.ends_with("/article"))
        .map(|(_, timeout)| *timeout)
        .expect("the seed URL was fetched");
    let deeper_timeout = seen
        .iter()
        .find(|(url, _)| url.ends_with("/deeper"))
        .map(|(_, timeout)| *timeout)
        .expect("the discovered link was fetched");
    // The budget is 5s. The discovered fetch happens after a ~1s crawl delay,
    // so it must be granted strictly less timeout than the deadline allows:
    // it can never be handed more than the time that actually remains.
    assert!(
        deeper_timeout < seed_timeout,
        "a fetch after a crawl delay must not inherit the pre-delay timeout: seed={seed_timeout} deeper={deeper_timeout}"
    );
    assert!(
        deeper_timeout <= 5_000,
        "the per-fetch timeout may never exceed the budgeted deadline: {deeper_timeout}"
    );
}

/// A fetcher that always fails with a fixed error, so a test can drive the Deep
/// error path and observe how the redirect budget is charged.
struct FailingFetcher(FetchError);

#[async_trait]
impl Fetcher for FailingFetcher {
    async fn fetch(&self, _: FetchRequest) -> Result<FetchResult, FetchError> {
        Err(self.0.clone())
    }
}

/// A fetcher that records the `max_redirects` it was handed for every request,
/// then follows (successfully) as many redirects as it was *allowed*, so a test
/// can prove the cap Deep hands each fetch is derived from the aggregate budget
/// that remains at that iteration.
struct MaxRedirectRecordingFetcher {
    seen: Arc<std::sync::Mutex<Vec<u32>>>,
}

#[async_trait]
impl Fetcher for MaxRedirectRecordingFetcher {
    async fn fetch(&self, request: FetchRequest) -> Result<FetchResult, FetchError> {
        self.seen.lock().unwrap().push(request.max_redirects);
        // A hop count equal to what was permitted: the fetch spends its whole
        // allowance while staying within the contract Deep set for it.
        let chain: Vec<Url> = (0..request.max_redirects)
            .map(|hop| Url::parse(&format!("https://example.com/hop-{hop}")).unwrap())
            .collect();
        let final_url = chain.last().cloned().unwrap_or_else(|| request.url.clone());
        Ok(FetchResult {
            final_url: amatl_core::FinalUrl(final_url),
            status: 200,
            headers_safe: BTreeMap::new(),
            content_type: Some("text/html".into()),
            body: b"<html><p>body</p></html>".to_vec(),
            size: 24,
            redirect_chain: chain,
            retrieved_at: "2026-08-14T00:00:00Z".into(),
        })
    }
}

fn deep_with_budget(fetch: FetchError) -> DeepOrchestrator {
    DeepOrchestrator::new(
        // fetches=4, bytes=100_000, redirects=5, browser=1, crawl=4, deadline=5s
        DeepBudget::new(4, 100_000, 5, 1, 4, 5_000).with_gap_limits(0, 0),
        Arc::new(FailingFetcher(fetch)),
        Arc::new(FixedExtractor {
            version: "fixed-v1",
            result: Ok(extraction_ok()),
        }),
        RendererPool::new(Arc::new(NoRenderer), 1),
        None,
        5_000,
        100_000,
        5,
        // top_k = 2 so the concurrency test can enqueue two candidates.
        2,
        0,
    )
}

#[tokio::test]
async fn a_fetch_that_failed_after_redirects_still_charges_its_redirects() {
    // Regression (B5): a fetch that follows redirects and then fails used to
    // drop the count entirely, because only the success path charged
    // `redirect_chain`. An origin could therefore walk an attacker-controlled
    // redirect chain, make the final hop fail, and repeat — spending real
    // network redirects while `remaining_redirects` never moved. The error now
    // carries the followed count and Deep debits it.
    let mut deep = deep_with_budget(FetchError::AfterRedirects {
        followed: 3,
        cause: Box::new(FetchError::Timeout),
    });
    assert_eq!(deep.budget_snapshot().remaining_redirects, 5);
    let _ = deep
        .enrich(request(vec![DeepCandidate {
            result: result(),
            storage_rights: false,
        }]))
        .await;
    assert_eq!(
        deep.budget_snapshot().remaining_redirects,
        2,
        "the three redirects followed before the failure must be charged"
    );
}

#[tokio::test]
async fn a_fetch_that_failed_before_any_redirect_charges_nothing() {
    // The mirror case: an error with no redirects followed must not touch the
    // redirect budget (no conservative "charge max_redirects" heuristic).
    let mut deep = deep_with_budget(FetchError::Timeout);
    assert_eq!(deep.budget_snapshot().remaining_redirects, 5);
    let _ = deep
        .enrich(request(vec![DeepCandidate {
            result: result(),
            storage_rights: false,
        }]))
        .await;
    assert_eq!(deep.budget_snapshot().remaining_redirects, 5);
}

#[tokio::test]
async fn a_failed_fetch_keeps_the_degradation_code_of_its_underlying_cause() {
    // Wrapping must not collapse classification: a blocked destination reached
    // *after* redirects still reports `fetch_blocked`, not a generic
    // "after redirects" code.
    let mut deep = deep_with_budget(FetchError::AfterRedirects {
        followed: 2,
        cause: Box::new(FetchError::AddressBlocked),
    });
    let output = deep
        .enrich(request(vec![DeepCandidate {
            result: result(),
            storage_rights: false,
        }]))
        .await;
    assert!(
        output
            .degradations
            .iter()
            .any(|degradation| degradation.code == "fetch_blocked"),
        "{:?}",
        output.degradations
    );
    assert_eq!(deep.budget_snapshot().remaining_redirects, 3);
}

#[tokio::test]
async fn reaching_the_redirect_limit_after_a_redirect_charges_only_the_followed_ones() {
    // `RedirectLimit` is raised when a redirect response is *received but not
    // followed*; the redirects already taken were pushed onto the chain and must
    // be charged, while the rejected one is not. Here three were followed.
    let mut deep = deep_with_budget(FetchError::AfterRedirects {
        followed: 3,
        cause: Box::new(FetchError::RedirectLimit),
    });
    let _ = deep
        .enrich(request(vec![DeepCandidate {
            result: result(),
            storage_rights: false,
        }]))
        .await;
    assert_eq!(deep.budget_snapshot().remaining_redirects, 2);
}

#[tokio::test]
async fn concurrent_failed_fetches_respect_the_aggregate_redirect_budget() {
    // Two candidates, each erroring after 3 redirects, against a redirect budget
    // of 5: the aggregate must never go negative, and the second charge is
    // rejected (charging 0) rather than wrapping the counter.
    let mut deep = deep_with_budget(FetchError::AfterRedirects {
        followed: 3,
        cause: Box::new(FetchError::Network),
    });
    let output = deep
        .enrich(request(vec![
            DeepCandidate {
                result: result(),
                storage_rights: false,
            },
            DeepCandidate {
                result: SearchResult {
                    canonical_url: CanonicalUrl(Url::parse("https://example.com/second").unwrap()),
                    original_url: OriginalUrl(Url::parse("https://example.com/second").unwrap()),
                    ..result()
                },
                storage_rights: false,
            },
        ]))
        .await;
    // Both fetches fail before producing a Document.
    assert!(output.documents.is_empty());
    // The first charge takes 5 -> 2; the second saturates 2 -> 0 rather than
    // underflowing, so the aggregate never exceeds its limit.
    assert_eq!(deep.budget_snapshot().remaining_redirects, 0);
}

#[tokio::test]
async fn each_fetch_is_capped_by_the_remaining_aggregate_redirect_budget() {
    // The global-redirect invariant is enforced *before* a fetch runs: Deep
    // hands every fetch `max_redirects = min(remaining_redirects, configured
    // cap)`, recomputed at the top of each iteration, so a fetch can never be
    // authorised to follow more redirects than the aggregate budget still
    // holds. Two candidates, one shared budget of 5: the first fetch is granted
    // all 5 and spends them (5 -> 0); the second is then granted 0, so the
    // aggregate is consumed exactly and never over-issued. This is the property
    // a concurrent design would have to re-establish under a lock; here it
    // holds because `enrich` drains the queue sequentially over `&mut self`.
    let seen = Arc::new(std::sync::Mutex::new(vec![]));
    let mut deep = DeepOrchestrator::new(
        DeepBudget::new(4, 100_000, 5, 1, 4, 5_000).with_gap_limits(0, 0),
        Arc::new(MaxRedirectRecordingFetcher { seen: seen.clone() }),
        Arc::new(FixedExtractor {
            version: "fixed-v1",
            result: Ok(extraction_ok()),
        }),
        RendererPool::new(Arc::new(NoRenderer), 1),
        None,
        5_000,
        100_000,
        // Configured per-fetch cap is 5, so the cap that actually binds is the
        // remaining aggregate budget, not this constant.
        5,
        2,
        0,
    );
    let output = deep
        .enrich(request(vec![
            DeepCandidate {
                result: result(),
                storage_rights: false,
            },
            DeepCandidate {
                result: SearchResult {
                    canonical_url: CanonicalUrl(Url::parse("https://example.com/second").unwrap()),
                    original_url: OriginalUrl(Url::parse("https://example.com/second").unwrap()),
                    ..result()
                },
                storage_rights: false,
            },
        ]))
        .await;
    assert_eq!(output.documents.len(), 2);
    // Fetches ran in queue order and each was handed the budget that remained
    // at its turn: the first all 5, the second none.
    assert_eq!(*seen.lock().unwrap(), vec![5, 0]);
    assert_eq!(deep.budget_snapshot().remaining_redirects, 0);
}

#[tokio::test]
async fn a_successful_chain_and_a_failed_chain_share_one_redirect_budget() {
    // Mixed outcome: the first candidate succeeds after spending 2 redirects
    // (charged via `redirect_chain`), the second fails after following 2 more
    // (charged via `followed_redirects`). Both debits land on one budget: 5 -> 3
    // -> 1, and the aggregate never exceeds the initial 5.
    struct MixedFetcher {
        calls: Arc<std::sync::atomic::AtomicUsize>,
    }
    #[async_trait]
    impl Fetcher for MixedFetcher {
        async fn fetch(&self, request: FetchRequest) -> Result<FetchResult, FetchError> {
            if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
                let hop = |n: u32| Url::parse(&format!("https://example.com/ok-{n}")).unwrap();
                Ok(FetchResult {
                    final_url: amatl_core::FinalUrl(hop(2)),
                    status: 200,
                    headers_safe: BTreeMap::new(),
                    content_type: Some("text/html".into()),
                    body: b"<html><p>body</p></html>".to_vec(),
                    size: 24,
                    redirect_chain: vec![request.url.clone(), hop(1)],
                    retrieved_at: "2026-08-14T00:00:00Z".into(),
                })
            } else {
                Err(FetchError::AfterRedirects {
                    followed: 2,
                    cause: Box::new(FetchError::Timeout),
                })
            }
        }
    }
    let mut deep = DeepOrchestrator::new(
        DeepBudget::new(4, 100_000, 5, 1, 4, 5_000).with_gap_limits(0, 0),
        Arc::new(MixedFetcher {
            calls: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        }),
        Arc::new(FixedExtractor {
            version: "fixed-v1",
            result: Ok(extraction_ok()),
        }),
        RendererPool::new(Arc::new(NoRenderer), 1),
        None,
        5_000,
        100_000,
        5,
        2,
        0,
    );
    let output = deep
        .enrich(request(vec![
            DeepCandidate {
                result: result(),
                storage_rights: false,
            },
            DeepCandidate {
                result: SearchResult {
                    canonical_url: CanonicalUrl(Url::parse("https://example.com/second").unwrap()),
                    original_url: OriginalUrl(Url::parse("https://example.com/second").unwrap()),
                    ..result()
                },
                storage_rights: false,
            },
        ]))
        .await;
    // The success and the failure each produced their own accounting without
    // one erasing the other: 5 - 2 (success chain) - 2 (failed chain) = 1.
    assert_eq!(output.documents.len(), 1);
    assert_eq!(output.errors.len(), 1);
    assert_eq!(deep.budget_snapshot().remaining_redirects, 1);
}

#[tokio::test]
async fn a_successful_fetch_reporting_more_redirects_than_remain_is_rejected() {
    // Defence in depth on the success path. `max_redirects` is already capped by
    // the remaining budget before the fetch runs, so a well-behaved fetcher can
    // never over-report. If one did anyway, `consume_fetch` must *reject* the
    // charge (ByteLimit/RedirectLimit) and Deep must drop the result rather than
    // debit more redirects than remain — the success-path mirror of the error
    // path's saturating `charge_redirects`. Without this guard a fetcher that
    // ignored its cap could drive the aggregate redirect spend past the initial
    // budget.
    struct OverReportingFetcher;
    #[async_trait]
    impl Fetcher for OverReportingFetcher {
        async fn fetch(&self, _: FetchRequest) -> Result<FetchResult, FetchError> {
            // Claims a 9-hop chain against a budget that only holds 2.
            let chain: Vec<Url> = (0..9)
                .map(|hop| Url::parse(&format!("https://example.com/hop-{hop}")).unwrap())
                .collect();
            Ok(FetchResult {
                final_url: amatl_core::FinalUrl(chain.last().unwrap().clone()),
                status: 200,
                headers_safe: BTreeMap::new(),
                content_type: Some("text/html".into()),
                body: b"<html><p>body</p></html>".to_vec(),
                size: 24,
                redirect_chain: chain,
                retrieved_at: "2026-08-14T00:00:00Z".into(),
            })
        }
    }
    let mut deep = DeepOrchestrator::new(
        // fetches=4, bytes=100_000, redirects=2, browser=1, crawl=4, deadline=5s
        DeepBudget::new(4, 100_000, 2, 1, 4, 5_000).with_gap_limits(0, 0),
        Arc::new(OverReportingFetcher),
        Arc::new(FixedExtractor {
            version: "fixed-v1",
            result: Ok(extraction_ok()),
        }),
        RendererPool::new(Arc::new(NoRenderer), 1),
        None,
        5_000,
        100_000,
        9,
        1,
        0,
    );
    let output = deep
        .enrich(request(vec![DeepCandidate {
            result: result(),
            storage_rights: false,
        }]))
        .await;
    // The over-charge was refused, so no Document was produced and the budget
    // was not debited past what it held.
    assert!(output.documents.is_empty());
    assert!(
        output
            .degradations
            .iter()
            .any(|degradation| degradation.code == "redirect_limit"),
        "{:?}",
        output.degradations
    );
    assert_eq!(deep.budget_snapshot().remaining_redirects, 2);
}
