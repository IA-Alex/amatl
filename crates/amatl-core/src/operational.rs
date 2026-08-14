use crate::{
    classify, parse_query, Budget, CacheStats, ChromiumRenderer, DeepBudget, DeepCandidate,
    DeepOrchestrator, DeepRequest, DocumentCache, ExtractError, ExtractionResult, Extractor,
    FetchError, FetchRequest, FetchResult, Fetcher, FinalUrl, MockBehavior, MockProvider, Provider,
    ProviderExecutionStatus, ProviderItem, ProviderResult, ProviderSearchCache,
    ProviderSearchCachePolicy, Rank, RendererPool, SearchOrchestrator, SearchPlan, SearchStatus,
    SqliteStorage, StorageError, SCHEMA_VERSION,
};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use thiserror::Error;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct LatencyPercentiles {
    pub samples: usize,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub p99_ms: f64,
    pub max_ms: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SearchOperationalReport {
    pub latency: LatencyPercentiles,
    pub throughput_requests_per_second: f64,
    pub success_rate: f64,
    pub partial_rate: f64,
    pub failure_rate: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SqliteOperationalReport {
    pub cold_write_latency: LatencyPercentiles,
    pub warm_read_latency: LatencyPercentiles,
    pub attempted_writes: usize,
    pub write_success_rate: f64,
    pub warm_hits: usize,
    pub warm_hit_rate: f64,
    pub cache_stats: CacheStats,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct OperationalBenchmarkReport {
    pub schema_version: String,
    pub workload: String,
    pub iterations: usize,
    pub concurrency: usize,
    pub search: SearchOperationalReport,
    pub deep_latency: LatencyPercentiles,
    pub sqlite: SqliteOperationalReport,
    pub peak_rss_bytes: Option<u64>,
}

#[derive(Debug, Error)]
pub enum OperationalBenchmarkError {
    #[error("operational benchmark storage failed")]
    Storage(#[from] StorageError),
    #[error("operational benchmark task failed")]
    Task,
}

pub async fn run_operational_benchmark(
    iterations: usize,
    concurrency: usize,
) -> Result<OperationalBenchmarkReport, OperationalBenchmarkError> {
    let iterations = iterations.clamp(1, 10_000);
    let concurrency = concurrency.clamp(1, 256);
    let search = benchmark_search(iterations, concurrency).await?;
    let deep_latency = benchmark_deep(iterations.min(32), concurrency).await?;
    let sqlite = benchmark_sqlite(iterations, concurrency).await?;
    Ok(OperationalBenchmarkReport {
        schema_version: SCHEMA_VERSION.into(),
        workload: "controlled-local-v1".into(),
        iterations,
        concurrency,
        search,
        deep_latency,
        sqlite,
        peak_rss_bytes: peak_rss_bytes(),
    })
}

async fn benchmark_search(
    iterations: usize,
    concurrency: usize,
) -> Result<SearchOperationalReport, OperationalBenchmarkError> {
    let semaphore = Arc::new(Semaphore::new(concurrency));
    let started = Instant::now();
    let mut tasks = JoinSet::new();
    for index in 0..iterations {
        let permit = semaphore
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| OperationalBenchmarkError::Task)?;
        tasks.spawn(async move {
            let _permit = permit;
            let providers: Vec<Arc<dyn Provider>> = vec![
                Arc::new(MockProvider::new(
                    "operational-success",
                    MockBehavior::Delayed(
                        vec![provider_item(&format!("https://result-{index}.example/"))],
                        2,
                    ),
                )),
                Arc::new(MockProvider::new(
                    "operational-failure",
                    MockBehavior::Failure(crate::ProviderErrorKind::Unavailable),
                )),
            ];
            let sample_started = Instant::now();
            let response = SearchOrchestrator::new(Budget::new(2, 1_000), 100)
                .with_execution_limits(2, 1, 0, 0)
                .search(
                    parse_query(format!("operational search {index}")).unwrap(),
                    providers,
                )
                .await;
            (sample_started.elapsed(), response.status)
        });
    }
    let mut latencies = Vec::with_capacity(iterations);
    let mut success = 0;
    let mut partial = 0;
    let mut failure = 0;
    while let Some(result) = tasks.join_next().await {
        let (latency, status) = result.map_err(|_| OperationalBenchmarkError::Task)?;
        latencies.push(latency);
        match status {
            SearchStatus::Success => success += 1,
            SearchStatus::PartialSuccess => partial += 1,
            SearchStatus::Failure => failure += 1,
        }
    }
    let elapsed = started.elapsed().as_secs_f64().max(f64::EPSILON);
    let denominator = iterations as f64;
    Ok(SearchOperationalReport {
        latency: percentiles(latencies),
        throughput_requests_per_second: iterations as f64 / elapsed,
        success_rate: success as f64 / denominator,
        partial_rate: partial as f64 / denominator,
        failure_rate: failure as f64 / denominator,
    })
}

async fn benchmark_deep(
    iterations: usize,
    concurrency: usize,
) -> Result<LatencyPercentiles, OperationalBenchmarkError> {
    let semaphore = Arc::new(Semaphore::new(concurrency));
    let mut tasks = JoinSet::new();
    for index in 0..iterations {
        let permit = semaphore
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| OperationalBenchmarkError::Task)?;
        tasks.spawn(async move {
            let _permit = permit;
            let provider: Arc<dyn Provider> = Arc::new(MockProvider::success(
                "operational-deep",
                vec![provider_item(&format!("https://deep-{index}.example/"))],
            ));
            let query = parse_query(format!("operational deep {index}")).unwrap();
            let mut search = SearchOrchestrator::new(Budget::new(1, 1_000), 100)
                .with_execution_limits(1, 1, 0, 0);
            let response = search.search(query.clone(), vec![provider]).await;
            let request = DeepRequest {
                query,
                search_plan: search.last_plan().cloned().unwrap(),
                candidates: response
                    .results
                    .into_iter()
                    .map(|result| DeepCandidate {
                        result,
                        storage_rights: false,
                    })
                    .collect(),
            };
            let started = Instant::now();
            let renderer_pool =
                RendererPool::new(Arc::new(ChromiumRenderer::detect(&Default::default())), 1);
            let response = DeepOrchestrator::new(
                DeepBudget::new(1, 1_048_576, 1, 1, 1, 1_000),
                Arc::new(StaticFetcher),
                Arc::new(StaticExtractor),
                renderer_pool,
                None::<DocumentCache>,
                1_000,
                1_048_576,
                1,
                1,
                0,
            )
            .enrich(request)
            .await;
            if response.documents.len() != 1 {
                return Err(OperationalBenchmarkError::Task);
            }
            Ok(started.elapsed())
        });
    }
    let mut latencies = Vec::with_capacity(iterations);
    while let Some(result) = tasks.join_next().await {
        latencies.push(result.map_err(|_| OperationalBenchmarkError::Task)??);
    }
    Ok(percentiles(latencies))
}

async fn benchmark_sqlite(
    iterations: usize,
    concurrency: usize,
) -> Result<SqliteOperationalReport, OperationalBenchmarkError> {
    let path = benchmark_database_path();
    let storage = SqliteStorage::open(&path).await?;
    let cache = ProviderSearchCache::new(
        storage,
        ProviderSearchCachePolicy {
            enabled: true,
            ttl_seconds: 300,
            max_entries: iterations as u64 + 1,
            max_bytes: 64 * 1024 * 1024,
        },
    );
    let fixtures = (0..iterations).map(cache_fixture).collect::<Vec<_>>();
    let (cold_write_latency, _) = cache_wave(&cache, &fixtures, concurrency, false).await?;
    let (warm_read_latency, warm_hits) = cache_wave(&cache, &fixtures, concurrency, true).await?;
    let cache_stats = cache.stats().await;
    drop(cache);
    remove_sqlite_files(&path);
    Ok(SqliteOperationalReport {
        cold_write_latency,
        warm_read_latency,
        attempted_writes: iterations,
        write_success_rate: cache_stats.entries as f64 / iterations as f64,
        warm_hits,
        warm_hit_rate: warm_hits as f64 / iterations as f64,
        cache_stats,
    })
}

async fn cache_wave(
    cache: &ProviderSearchCache,
    fixtures: &[(SearchPlan, ProviderResult)],
    concurrency: usize,
    read: bool,
) -> Result<(LatencyPercentiles, usize), OperationalBenchmarkError> {
    let semaphore = Arc::new(Semaphore::new(concurrency));
    let mut tasks = JoinSet::new();
    for (plan, result) in fixtures {
        let permit = semaphore
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| OperationalBenchmarkError::Task)?;
        let cache = cache.clone();
        let plan = plan.clone();
        let result = result.clone();
        tasks.spawn(async move {
            let _permit = permit;
            let started = Instant::now();
            let hit = if read {
                cache.get("operational-cache", "v1", &plan).await.is_some()
            } else {
                cache.put("operational-cache", "v1", &plan, &result).await;
                false
            };
            (started.elapsed(), hit)
        });
    }
    let mut latencies = Vec::with_capacity(fixtures.len());
    let mut hits = 0;
    while let Some(result) = tasks.join_next().await {
        let (latency, hit) = result.map_err(|_| OperationalBenchmarkError::Task)?;
        latencies.push(latency);
        hits += usize::from(hit);
    }
    Ok((percentiles(latencies), hits))
}

fn cache_fixture(index: usize) -> (SearchPlan, ProviderResult) {
    let query = parse_query(format!("sqlite contention {index}")).unwrap();
    let provider: Arc<dyn Provider> = Arc::new(MockProvider::success(
        "operational-cache",
        vec![provider_item(&format!("https://cache-{index}.example/"))],
    ));
    let mut orchestrator = SearchOrchestrator::new(Budget::new(1, 1_000), 100);
    let plan = orchestrator.plan(query.clone(), classify(&query), &[provider]);
    let result = ProviderResult {
        schema_version: SCHEMA_VERSION.into(),
        provider: "operational-cache".into(),
        status: ProviderExecutionStatus::Success,
        results: vec![provider_item(&format!("https://cache-{index}.example/"))],
        accepted_filters: vec![],
        ignored_filters: vec![],
        approximated_filters: vec![],
        errors: vec![],
    };
    (plan, result)
}

fn provider_item(url: &str) -> ProviderItem {
    ProviderItem {
        title: Some("Controlled operational result".into()),
        url: url.into(),
        provider_rank: Some(Rank::FIRST),
        snippet: Some("deterministic local workload".into()),
        result_type: None,
        published_at: None,
        author: None,
        language: None,
        file_type: None,
        thumbnail: None,
        metadata: BTreeMap::new(),
    }
}

fn percentiles(mut values: Vec<Duration>) -> LatencyPercentiles {
    values.sort_unstable();
    let at = |quantile: f64| {
        let index = ((values.len().saturating_sub(1)) as f64 * quantile).ceil() as usize;
        values[index].as_secs_f64() * 1_000.0
    };
    LatencyPercentiles {
        samples: values.len(),
        p50_ms: at(0.50),
        p95_ms: at(0.95),
        p99_ms: at(0.99),
        max_ms: at(1.0),
    }
}

fn benchmark_database_path() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "amatl-operational-{}-{nonce}.sqlite3",
        std::process::id()
    ))
}

fn remove_sqlite_files(path: &std::path::Path) {
    let _ = std::fs::remove_file(path);
    for suffix in ["-shm", "-wal"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
    }
}

fn peak_rss_bytes() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let kibibytes = status
        .lines()
        .find_map(|line| line.strip_prefix("VmHWM:"))?
        .split_whitespace()
        .next()?
        .parse::<u64>()
        .ok()?;
    kibibytes.checked_mul(1024)
}

struct StaticFetcher;

#[async_trait]
impl Fetcher for StaticFetcher {
    async fn fetch(&self, request: FetchRequest) -> Result<FetchResult, FetchError> {
        tokio::time::sleep(Duration::from_millis(1)).await;
        let body = b"<html><body><main>controlled deep content</main></body></html>".to_vec();
        Ok(FetchResult {
            final_url: FinalUrl(request.url),
            status: 200,
            headers_safe: BTreeMap::new(),
            content_type: Some("text/html".into()),
            size: body.len() as u64,
            body,
            redirect_chain: vec![],
            retrieved_at: "2026-01-01T00:00:00Z".into(),
        })
    }
}

struct StaticExtractor;

#[async_trait]
impl Extractor for StaticExtractor {
    fn name(&self) -> &str {
        "operational-static"
    }

    fn version(&self) -> &str {
        "v1"
    }

    async fn extract(&self, _: &[u8]) -> Result<ExtractionResult, ExtractError> {
        Ok(ExtractionResult {
            content: "controlled deep content".into(),
            format: "text".into(),
            title: Some("Controlled Deep Document".into()),
            author: None,
            published_at: None,
            metadata: BTreeMap::new(),
            extractor_used: "operational-static-v1".into(),
            status: "success".into(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn controlled_operational_report_is_complete_and_bounded() {
        let report = run_operational_benchmark(4, 2).await.unwrap();
        assert_eq!(report.schema_version, SCHEMA_VERSION);
        assert_eq!(report.search.latency.samples, 4);
        assert_eq!(report.deep_latency.samples, 4);
        assert_eq!(report.search.partial_rate, 1.0);
        assert_eq!(report.search.failure_rate, 0.0);
        assert_eq!(report.sqlite.warm_hits, 4);
        assert_eq!(report.sqlite.cache_stats.entries, 4);
        assert_eq!(report.sqlite.write_success_rate, 1.0);
        assert_eq!(report.sqlite.warm_hit_rate, 1.0);
    }
}
