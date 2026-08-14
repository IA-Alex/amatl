use crate::cache::{CacheCounters, CacheEffectiveness};
use crate::storage::{CacheStats, SavedDocument, SearchHistoryEntry};
use crate::telemetry::{now_unix, ProviderValueSnapshot};
use crate::{
    parse_query, Budget, CachedProvider, ChromiumRenderer, Config, DeepBudget, DeepCandidate,
    DeepOrchestrator, DeepRequest, DeepResponse, DocumentCache, DocumentCachePolicy, ErrorCode,
    GapAnalyzer, InMemoryTelemetry, InferenceRuntime, MockProvider, Provider, ProviderAvailability,
    ProviderBuildContext, ProviderCapabilities, ProviderItem, ProviderRegistry,
    ProviderRuntimeConfig, ProviderSearchCache, ProviderSearchCachePolicy, Query, Rank,
    RankingV2Engine, RendererPool, ReqwestTransport, SafeFetcher, SearchOrchestrator, SearchPlan,
    SearchResponse, SearchSubQueryExecutor, SqliteStorage, StorageError, TrafilaturaExtractor,
    SCHEMA_VERSION,
};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use thiserror::Error;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServiceSurface {
    pub kind: ServiceSurfaceKind,
    /// Correlates this invocation with the originating HTTP request, CLI
    /// invocation, or MCP session so traces can be reconstructed end-to-end.
    pub request_id: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServiceSurfaceKind {
    Cli,
    Api,
    Mcp,
}

impl ServiceSurface {
    pub fn cli() -> Self {
        Self {
            kind: ServiceSurfaceKind::Cli,
            request_id: None,
        }
    }

    pub fn api(request_id: Option<String>) -> Self {
        Self {
            kind: ServiceSurfaceKind::Api,
            request_id,
        }
    }

    pub fn mcp() -> Self {
        Self::mcp_with_request_id(None)
    }

    /// MCP surface correlated with the tool call that produced it.
    pub fn mcp_with_request_id(request_id: Option<String>) -> Self {
        Self {
            kind: ServiceSurfaceKind::Mcp,
            request_id,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExecutionLimits {
    pub max_provider_calls: u32,
    pub provider_timeout_ms: u64,
    pub search_timeout_ms: u64,
    pub deep_max_fetches: u32,
    pub deep_max_bytes: u64,
    pub deep_timeout_ms: u64,
    pub deep_max_subqueries: u32,
    pub deep_max_subquery_cost: u64,
}

impl ExecutionLimits {
    pub fn for_surface(config: &Config, surface: ServiceSurface) -> Self {
        let base = Self {
            max_provider_calls: config.budget.max_provider_calls,
            provider_timeout_ms: config.timeouts.provider_ms,
            search_timeout_ms: config.timeouts.global_ms,
            deep_max_fetches: config.deep.max_fetches,
            deep_max_bytes: config.deep.max_bytes,
            deep_timeout_ms: config.deep.timeout_ms,
            deep_max_subqueries: config.deep.gaps.max_subqueries,
            deep_max_subquery_cost: config.deep.gaps.max_cost,
        };
        match surface.kind {
            ServiceSurfaceKind::Cli | ServiceSurfaceKind::Api => base,
            ServiceSurfaceKind::Mcp => Self {
                max_provider_calls: base.max_provider_calls.min(2),
                provider_timeout_ms: base.provider_timeout_ms.min(2_500),
                search_timeout_ms: base.search_timeout_ms.min(5_000),
                deep_max_fetches: base.deep_max_fetches.min(3),
                deep_max_bytes: base.deep_max_bytes.min(2 * 1024 * 1024),
                deep_timeout_ms: base.deep_timeout_ms.min(10_000),
                deep_max_subqueries: base.deep_max_subqueries.min(1),
                deep_max_subquery_cost: base.deep_max_subquery_cost.min(1),
            },
        }
    }
}

#[derive(Clone, Debug)]
pub struct SearchExecution {
    pub query: Query,
    pub plan: SearchPlan,
    pub response: SearchResponse,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProviderSurfaceStatus {
    Available,
    Unavailable,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderSummary {
    pub schema_version: String,
    pub name: String,
    pub status: ProviderSurfaceStatus,
    pub code: Option<String>,
    pub capabilities: ProviderCapabilities,
}

/// Domain failures a surface can receive from the service.
///
/// Every variant maps to exactly one [`ErrorCode`], so CLI, API and MCP report
/// the same cause with the same wire identifier instead of collapsing distinct
/// failures into one generic code.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum ServiceError {
    #[error("invalid query")]
    InvalidQuery,
    #[error("search planning failed")]
    MissingPlan,
    #[error("service configuration failed")]
    Configuration,
    #[error("enabled provider has no governance record: {0}")]
    ProviderNotDeclared(String),
    #[error("declared provider has no registered implementation: {0}")]
    ProviderNotRegistered(String),
    #[error("ranking backend is unavailable")]
    RankingBackend,
    #[error("required inference backend is unavailable")]
    InferenceUnavailable,
    #[error("local persistence is unavailable")]
    StorageUnavailable,
    #[error("request payload is invalid")]
    InvalidInput,
}

impl ServiceError {
    /// Catalog entry for this failure.
    pub fn code(&self) -> ErrorCode {
        match self {
            Self::InvalidQuery => ErrorCode::InvalidQuery,
            Self::MissingPlan => ErrorCode::SearchPlanningFailed,
            Self::Configuration => ErrorCode::ConfigurationInvalid,
            Self::ProviderNotDeclared(_) => ErrorCode::ProviderNotDeclared,
            Self::ProviderNotRegistered(_) => ErrorCode::ProviderNotRegistered,
            Self::RankingBackend => ErrorCode::RankingBackendUnavailable,
            Self::InferenceUnavailable => ErrorCode::InferenceUnavailable,
            Self::StorageUnavailable => ErrorCode::StorageUnavailable,
            Self::InvalidInput => ErrorCode::InvalidRequest,
        }
    }
}

/// Payload a surface may persist as a saved document.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct SaveDocumentInput {
    pub canonical_url: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub snippet: Option<String>,
    pub content_hash: String,
    pub extractor_version: String,
    pub payload: String,
    #[serde(default)]
    pub source_query: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
}

/// Availability, persistence and cache state of one running service.
///
/// This is an operator view: it aggregates what the service already knows
/// instead of recomputing anything, and it never exposes ranking internals.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ServiceStatus {
    pub schema_version: String,
    /// `ok` when every declared source is available and persistence is healthy
    /// (or disabled by configuration); `degraded` otherwise.
    pub status: String,
    pub sources: Vec<SourceStatus>,
    pub storage: StorageStatus,
    pub cache: CacheStatus,
    pub inference_backend: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SourceStatus {
    pub name: String,
    pub status: ProviderSurfaceStatus,
    pub code: Option<String>,
    /// Observed success ratio in the telemetry window, when there are samples.
    pub success_rate: Option<f64>,
    pub average_latency_ms: Option<f64>,
    pub observations: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct StorageStatus {
    pub enabled: bool,
    pub available: bool,
    pub migration_version: Option<i64>,
    pub history_entries: Option<i64>,
    pub saved_documents: Option<i64>,
    pub message: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CacheStatus {
    pub provider_search_enabled: bool,
    pub document_enabled: bool,
    pub provider_search_entries: u64,
    pub provider_search_bytes: u64,
    pub document_entries: u64,
    pub document_bytes: u64,
    #[serde(flatten)]
    pub effectiveness: CacheEffectiveness,
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum ProviderCanaryError {
    #[error("network egress is denied by data policy")]
    EgressDenied,
    #[error("unknown provider: {0}")]
    UnknownProvider(String),
    #[error("provider is not enabled: {0}")]
    NotEnabled(String),
    #[error("provider governance approval is incomplete or expired: {0}")]
    Governance(String),
    #[error("provider credential environment variable is missing or empty: {0}")]
    Credential(String),
    #[error("provider has no authorized real-network canary: {0}")]
    NetworkBlocked(String),
}

impl ProviderCanaryError {
    /// Catalog entry for this failure.
    pub fn code(&self) -> ErrorCode {
        match self {
            Self::EgressDenied => ErrorCode::EgressDenied,
            Self::UnknownProvider(_) => ErrorCode::ProviderNotRegistered,
            Self::NotEnabled(_) => ErrorCode::ProviderNotEnabled,
            Self::Governance(_) => ErrorCode::ProviderNotApproved,
            Self::Credential(_) => ErrorCode::ProviderCredentialMissing,
            Self::NetworkBlocked(_) => ErrorCode::ProviderNetworkBlocked,
        }
    }
}

#[derive(Clone)]
pub struct AmatlService {
    config: Arc<Config>,
    registry: Arc<ProviderRegistry>,
    storage: Option<SqliteStorage>,
    storage_degradation: Option<crate::Degradation>,
    telemetry: InMemoryTelemetry,
    transport: Option<Arc<dyn crate::HttpTransport>>,
    fetcher: Arc<dyn crate::Fetcher>,
    inference: Option<InferenceRuntime>,
    mock: bool,
    /// Renderer pool created once at startup and reused across deep requests.
    renderer_pool: RendererPool,
    /// Shared cache hit/miss counters for the operator status surface.
    cache_counters: Arc<CacheCounters>,
}

impl AmatlService {
    /// Service backed by the providers AMATL ships.
    pub async fn new(config: Config, mock: bool) -> Self {
        Self::with_registry(config, mock, ProviderRegistry::builtin()).await
    }

    /// Service backed by a caller-supplied registry, so an embedder can add or
    /// remove sources without changing the core.
    pub async fn with_registry(config: Config, mock: bool, registry: ProviderRegistry) -> Self {
        let (storage, storage_degradation) = if config.persistence.enabled {
            match SqliteStorage::open(&config.persistence.path).await {
                Ok(storage) => (Some(storage), None),
                Err(error) => {
                    let message = storage_failure_message(&error);
                    tracing::warn!(
                        target: "amatl::storage",
                        error = %error,
                        path = %config.persistence.path,
                        quarantine_path = error.quarantine_path().map(|path| path.display().to_string()),
                        "SQLite persistence is unavailable; continuing without persistent cache or telemetry"
                    );
                    (
                        None,
                        Some(crate::Degradation {
                            code: ErrorCode::StorageUnavailable.as_str().into(),
                            component: "sqlite".into(),
                            message,
                        }),
                    )
                }
            }
        } else {
            (None, None)
        };
        let telemetry = InMemoryTelemetry::with_storage_and_retention(
            config
                .telemetry
                .persistence_enabled
                .then(|| storage.clone())
                .flatten(),
            config.telemetry.retention_days,
        )
        .await;
        let transport: Option<Arc<dyn crate::HttpTransport>> =
            if config.data_policy.allows_network_egress() {
                ReqwestTransport::new(2 * 1024 * 1024)
                    .map(|transport| Arc::new(transport) as Arc<dyn crate::HttpTransport>)
                    .map_err(|error| {
                        tracing::warn!(
                            target: "amatl::providers",
                            error = %error,
                            "provider HTTP transport could not be initialized"
                        );
                        error
                    })
                    .ok()
            } else {
                Some(Arc::new(DeniedHttpTransport))
            };
        let fetcher: Arc<dyn crate::Fetcher> = if config.data_policy.allows_network_egress() {
            Arc::new(SafeFetcher::default())
        } else {
            Arc::new(DeniedFetcher)
        };
        let inference = match InferenceRuntime::from_policy(&config.data_policy, &config.inference)
        {
            Ok(runtime) => runtime,
            Err(error) => {
                tracing::warn!(
                    target: "amatl::inference",
                    error = %error,
                    mode = config.data_policy.inference.as_str(),
                    "inference backend is unavailable; ranking stays lexical"
                );
                None
            }
        };
        let renderer = Arc::new(ChromiumRenderer::detect(&config.deep.renderer));
        let renderer_pool =
            RendererPool::new(renderer, config.deep.renderer.max_browser_calls as usize);
        Self {
            config: Arc::new(config),
            registry: Arc::new(registry),
            storage,
            storage_degradation,
            telemetry,
            transport,
            fetcher,
            inference,
            mock,
            renderer_pool,
            cache_counters: Arc::new(CacheCounters::default()),
        }
    }

    /// Registry backing this service.
    pub fn registry(&self) -> &ProviderRegistry {
        &self.registry
    }

    /// Identifier of the active inference backend, when one is available.
    pub fn inference_backend(&self) -> Option<&str> {
        self.inference.as_ref().map(InferenceRuntime::backend_id)
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    pub fn storage(&self) -> Option<SqliteStorage> {
        self.storage.clone()
    }

    pub async fn fetch_public(
        &self,
        request: crate::FetchRequest,
    ) -> Result<crate::FetchResult, crate::FetchError> {
        self.fetcher.fetch(request).await
    }

    pub async fn search(
        &self,
        raw_query: String,
        surface: ServiceSurface,
    ) -> Result<SearchExecution, ServiceError> {
        self.search_paginated(raw_query, surface, None, None).await
    }

    /// Search with optional server-side pagination.
    ///
    /// Every page is served from a full execution, so `page` selects a window
    /// over the ranked result set and `total_results` always describes the
    /// whole set. Surfaces must not mix this with client-side windowing.
    pub async fn search_paginated(
        &self,
        raw_query: String,
        surface: ServiceSurface,
        page: Option<u32>,
        page_size: Option<u32>,
    ) -> Result<SearchExecution, ServiceError> {
        let execution = self
            .search_inner(raw_query, surface.clone(), page, page_size)
            .await?;
        self.record_history(&execution, 0, surface).await;
        Ok(execution)
    }

    async fn search_inner(
        &self,
        raw_query: String,
        surface: ServiceSurface,
        page: Option<u32>,
        page_size: Option<u32>,
    ) -> Result<SearchExecution, ServiceError> {
        let query = parse_query(raw_query).map_err(|_| ServiceError::InvalidQuery)?;
        let limits = ExecutionLimits::for_surface(&self.config, surface.clone());
        let providers = self.providers()?;
        let mut orchestrator = SearchOrchestrator::new(
            Budget::new(limits.max_provider_calls, limits.search_timeout_ms),
            limits.provider_timeout_ms,
        )
        .with_execution_limits(
            self.config.execution.global_concurrency,
            self.config.execution.per_provider_concurrency,
            self.config.execution.max_retries,
            self.config.execution.retry_jitter_ms,
        )
        .with_result_policies(
            self.config.ranking_policy.clone(),
            self.config.diversity_policy.clone(),
        )
        .with_search_policy(self.config.search_policy.clone())
        .with_telemetry(self.telemetry.clone())
        .with_request_id(surface.request_id.clone());
        let mut response = orchestrator.search(query.clone(), providers).await;
        if let Some(degradation) = &self.storage_degradation {
            response.degradations.push(degradation.clone());
            if response.status == crate::SearchStatus::Success {
                response.status = crate::SearchStatus::PartialSuccess;
            }
        }
        // Apply server-side pagination when requested.
        if let (Some(p), Some(ps)) = (page, page_size) {
            let ps = ps.clamp(1, 100);
            let total = response.results.len() as u64;
            let start = (p as usize).saturating_mul(ps as usize);
            let end = start
                .saturating_add(ps as usize)
                .min(response.results.len());
            response.results = if start < response.results.len() {
                response.results[start..end].to_vec()
            } else {
                vec![]
            };
            response.total_results = Some(total);
            response.page = Some(p);
            response.page_size = Some(ps);
        }
        let plan = orchestrator
            .last_plan()
            .cloned()
            .ok_or(ServiceError::MissingPlan)?;
        Ok(SearchExecution {
            query,
            plan,
            response,
        })
    }

    pub async fn deep(
        &self,
        raw_query: String,
        surface: ServiceSurface,
    ) -> Result<DeepResponse, ServiceError> {
        let search = self
            .search_inner(raw_query, surface.clone(), None, None)
            .await?;
        let history_execution = search.clone();
        let limits = ExecutionLimits::for_surface(&self.config, surface.clone());
        let document_cache = self.storage.clone().and_then(|storage| {
            self.config.cache.document.enabled.then(|| {
                DocumentCache::new(
                    storage,
                    DocumentCachePolicy {
                        enabled: self.config.cache.document.enabled,
                        ttl_seconds: self.config.cache.document.ttl_seconds,
                        max_entries: self.config.cache.document.max_entries,
                        max_bytes: self.config.cache.document.max_bytes,
                        store_content: self.config.cache.document.store_content,
                        stale_while_revalidate_seconds: self
                            .config
                            .cache
                            .document
                            .stale_while_revalidate_seconds,
                    },
                )
                .with_counters(self.cache_counters.clone())
            })
        });
        let candidates = search
            .response
            .results
            .iter()
            .cloned()
            .map(|result| {
                let storage_rights = !self.mock
                    && result
                        .providers
                        .iter()
                        .all(|provider| self.config.providers.storage_rights(provider));
                DeepCandidate {
                    result,
                    storage_rights,
                }
            })
            .collect();
        let remaining_deep_ms = limits
            .deep_timeout_ms
            .saturating_sub(search.response.elapsed_ms);
        let deep_budget = DeepBudget::new(
            limits.deep_max_fetches,
            limits.deep_max_bytes,
            self.config.deep.max_redirects,
            self.config.deep.renderer.max_browser_calls,
            self.config.deep.max_crawl_urls,
            remaining_deep_ms,
        )
        .with_gap_limits(limits.deep_max_subqueries, limits.deep_max_subquery_cost);
        let extractor = Arc::new(TrafilaturaExtractor::new(
            self.config.deep.extractor.executable.clone(),
            self.config.deep.extractor.version.clone(),
            self.config.deep.extractor.timeout_ms.min(remaining_deep_ms),
            self.config.deep.extractor.max_output_bytes,
        ));
        let mut orchestrator = DeepOrchestrator::new(
            deep_budget,
            self.fetcher.clone(),
            extractor,
            self.renderer_pool.clone(),
            document_cache,
            remaining_deep_ms,
            (limits.deep_max_bytes / u64::from(limits.deep_max_fetches)).max(1),
            self.config.deep.max_redirects,
            self.config.deep.top_k as usize,
            self.config.deep.max_depth,
        )
        .with_request_id(surface.request_id.clone());
        let mut pending_degradations = Vec::new();
        if self.config.deep.ranking_v2.enabled {
            let policy = self.config.deep.ranking_v2.policy.clone();
            let needs_semantic = policy.weight_semantic > 0.0;
            let needs_reranker = policy.weight_reranker > 0.0;
            let mut engine =
                RankingV2Engine::new(policy).map_err(|_| ServiceError::Configuration)?;
            if needs_semantic || needs_reranker {
                match &self.inference {
                    Some(runtime) => {
                        if needs_semantic {
                            engine = engine.with_semantic_scorer(runtime.semantic_scorer());
                        }
                        if needs_reranker {
                            engine = engine.with_reranker(
                                runtime
                                    .reranker()
                                    .map_err(|_| ServiceError::InferenceUnavailable)?,
                            );
                        }
                    }
                    None => pending_degradations.push(crate::Degradation {
                        code: crate::ErrorCode::InferenceUnavailable.as_str().into(),
                        component: "inference".into(),
                        message:
                            "Inference backend is unavailable; ranking used lexical signals only"
                                .into(),
                    }),
                }
            }
            orchestrator = orchestrator.with_ranking_v2(engine);
        }
        if self.config.deep.gaps.enabled && limits.deep_max_subqueries > 0 {
            let analyzer = GapAnalyzer::new(self.config.deep.gaps.policy.clone())
                .map_err(|_| ServiceError::Configuration)?;
            let executor = SearchSubQueryExecutor::new(
                self.providers()?,
                self.config
                    .deep
                    .gaps
                    .max_provider_calls_per_subquery
                    .min(limits.max_provider_calls),
                limits
                    .provider_timeout_ms
                    .min(self.config.deep.gaps.timeout_ms),
                self.config.deep.gaps.timeout_ms.min(remaining_deep_ms),
                self.config.ranking_policy.clone(),
                self.config.diversity_policy.clone(),
                self.config.search_policy.clone(),
            );
            orchestrator = orchestrator
                .with_gap_analyzer(analyzer)
                .with_subquery_executor(Arc::new(executor));
        }
        let mut response = orchestrator
            .enrich(DeepRequest {
                query: search.query,
                search_plan: search.plan,
                candidates,
            })
            .await;
        response.degradations.extend(pending_degradations);
        if let Some(degradation) = &self.storage_degradation {
            response.degradations.push(degradation.clone());
        }
        self.record_history(&history_execution, response.documents.len() as i64, surface)
            .await;
        Ok(response)
    }

    // ── Local domain surfaces: history, saved documents and status ──────

    /// Record one executed search in the local history, best effort.
    ///
    /// Failures never affect the search result: history is an operator
    /// convenience, not part of the Search contract.
    async fn record_history(
        &self,
        execution: &SearchExecution,
        deep_fetches: i64,
        surface: ServiceSurface,
    ) {
        if !self.config.persistence.history_enabled {
            return;
        }
        let Some(storage) = &self.storage else { return };
        let surface_name = match surface.kind {
            ServiceSurfaceKind::Cli => "cli",
            ServiceSurfaceKind::Api => "api",
            ServiceSurfaceKind::Mcp => "mcp",
        };
        let total_results = execution
            .response
            .total_results
            .map(|value| value as i64)
            .unwrap_or(execution.response.results.len() as i64);
        if storage
            .search_history_insert(
                &execution.query.normalized_query,
                &execution.query.raw_query,
                Some(crate::telemetry::category_name(
                    &execution.plan.classification.primary_category,
                )),
                execution.plan.selected_providers.len() as i64,
                total_results,
                deep_fetches,
                surface_name,
            )
            .await
            .is_err()
        {
            tracing::warn!(
                target: "amatl::storage",
                request_id = surface.request_id.as_deref().unwrap_or("-"),
                "search history entry could not be recorded"
            );
        }
    }

    fn storage_or_error(&self) -> Result<&SqliteStorage, ServiceError> {
        self.storage
            .as_ref()
            .ok_or(ServiceError::StorageUnavailable)
    }

    /// Recent searches, newest first.
    pub async fn history(
        &self,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<SearchHistoryEntry>, ServiceError> {
        self.storage_or_error()?
            .search_history_list(limit.clamp(1, 200) as i64, offset as i64)
            .await
            .map_err(|_| ServiceError::StorageUnavailable)
    }

    /// Delete one history entry; `false` when it did not exist.
    pub async fn delete_history_entry(&self, id: i64) -> Result<bool, ServiceError> {
        self.storage_or_error()?
            .search_history_delete(id)
            .await
            .map_err(|_| ServiceError::StorageUnavailable)
    }

    /// Delete every history entry, returning how many were removed.
    pub async fn purge_history(&self) -> Result<u64, ServiceError> {
        self.storage_or_error()?
            .search_history_purge()
            .await
            .map_err(|_| ServiceError::StorageUnavailable)
    }

    /// Saved documents, newest first.
    pub async fn saved_documents(
        &self,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<SavedDocument>, ServiceError> {
        self.storage_or_error()?
            .saved_document_list(limit.clamp(1, 200) as i64, offset as i64)
            .await
            .map_err(|_| ServiceError::StorageUnavailable)
    }

    /// Persist one document for cross-session reuse.
    ///
    /// The input is validated before it reaches SQLite: the URL must be
    /// public HTTP(S), the content hash must be a SHA-256 digest and the
    /// payload must fit `persistence.saved_document_max_bytes`.
    pub async fn save_document(&self, input: SaveDocumentInput) -> Result<i64, ServiceError> {
        let storage = self.storage_or_error()?;
        let url = url::Url::parse(&input.canonical_url).map_err(|_| ServiceError::InvalidInput)?;
        if !matches!(url.scheme(), "http" | "https") {
            return Err(ServiceError::InvalidInput);
        }
        if !is_sha256(&input.content_hash) {
            return Err(ServiceError::InvalidInput);
        }
        if input.extractor_version.is_empty() || input.extractor_version.len() > 128 {
            return Err(ServiceError::InvalidInput);
        }
        if input.payload.len() as u64 > self.config.persistence.saved_document_max_bytes {
            return Err(ServiceError::InvalidInput);
        }
        if input.tags.len() > 16 || input.tags.iter().any(|tag| tag.len() > 64) {
            return Err(ServiceError::InvalidInput);
        }
        let tags = input.tags.join(",");
        storage
            .saved_document_put(
                url.as_str(),
                input
                    .title
                    .as_deref()
                    .map(|value| bounded(value, 300))
                    .as_deref(),
                input
                    .snippet
                    .as_deref()
                    .map(|value| bounded(value, 2_000))
                    .as_deref(),
                &input.content_hash,
                &input.extractor_version,
                &input.payload,
                input
                    .source_query
                    .as_deref()
                    .map(|value| bounded(value, 2_048))
                    .as_deref(),
                &tags,
            )
            .await
            .map_err(|_| ServiceError::StorageUnavailable)
    }

    /// Delete one saved document; `false` when it did not exist.
    pub async fn delete_saved_document(&self, id: i64) -> Result<bool, ServiceError> {
        self.storage_or_error()?
            .saved_document_delete(id)
            .await
            .map_err(|_| ServiceError::StorageUnavailable)
    }

    /// Cache hit/miss counters accumulated since start.
    pub fn cache_effectiveness(&self) -> CacheEffectiveness {
        self.cache_counters.snapshot()
    }

    /// Observed value snapshots per source, used by metrics and status.
    pub fn source_snapshots(&self) -> Vec<ProviderValueSnapshot> {
        self.telemetry.snapshots(now_unix())
    }

    /// Operator status: source availability, persistence and cache state.
    pub async fn status(&self) -> Result<ServiceStatus, ServiceError> {
        let summaries = self.provider_summaries()?;
        let snapshots = self.source_snapshots();
        let sources = summaries
            .into_iter()
            .map(|summary| {
                let snapshot = snapshots
                    .iter()
                    .find(|snapshot| snapshot.provider == summary.name);
                SourceStatus {
                    name: summary.name,
                    status: summary.status,
                    code: summary.code,
                    success_rate: snapshot
                        .filter(|snapshot| snapshot.sample > 0)
                        .map(|snapshot| snapshot.success_rate),
                    average_latency_ms: snapshot
                        .filter(|snapshot| snapshot.sample > 0)
                        .map(|snapshot| snapshot.average_latency_ms),
                    observations: snapshot.map(|snapshot| snapshot.sample).unwrap_or(0),
                }
            })
            .collect::<Vec<_>>();
        let storage = self.storage_status().await;
        let (provider_search_stats, document_stats) = self.cache_stats().await;
        let cache = CacheStatus {
            provider_search_enabled: self.config.cache.provider_search.enabled,
            document_enabled: self.config.cache.document.enabled,
            provider_search_entries: provider_search_stats.entries,
            provider_search_bytes: provider_search_stats.size_bytes,
            document_entries: document_stats.entries,
            document_bytes: document_stats.size_bytes,
            effectiveness: self.cache_effectiveness(),
        };
        let degraded = storage.enabled && !storage.available
            || sources
                .iter()
                .any(|source| source.status != ProviderSurfaceStatus::Available);
        Ok(ServiceStatus {
            schema_version: SCHEMA_VERSION.into(),
            status: if degraded { "degraded" } else { "ok" }.into(),
            sources,
            storage,
            cache,
            inference_backend: self.inference_backend().map(str::to_owned),
        })
    }

    async fn storage_status(&self) -> StorageStatus {
        let enabled = self.config.persistence.enabled;
        let Some(storage) = &self.storage else {
            return StorageStatus {
                enabled,
                available: false,
                migration_version: None,
                history_entries: None,
                saved_documents: None,
                message: self
                    .storage_degradation
                    .as_ref()
                    .map(|degradation| degradation.message.clone()),
            };
        };
        StorageStatus {
            enabled,
            available: true,
            migration_version: storage
                .health()
                .await
                .ok()
                .map(|health| health.migration_version),
            history_entries: storage.search_history_count().await.ok(),
            saved_documents: storage.saved_document_count().await.ok(),
            message: None,
        }
    }

    async fn cache_stats(&self) -> (CacheStats, CacheStats) {
        let Some(storage) = self.storage.clone() else {
            return (CacheStats::default(), CacheStats::default());
        };
        let provider_search = ProviderSearchCache::new(
            storage.clone(),
            ProviderSearchCachePolicy {
                enabled: self.config.cache.provider_search.enabled,
                ..Default::default()
            },
        )
        .stats()
        .await;
        let document = DocumentCache::new(
            storage,
            DocumentCachePolicy {
                enabled: self.config.cache.document.enabled,
                ..Default::default()
            },
        )
        .stats()
        .await;
        (provider_search, document)
    }

    /// Availability of every declared provider that has an implementation.
    pub fn provider_summaries(&self) -> Result<Vec<ProviderSummary>, ServiceError> {
        let transport = self.transport.clone().ok_or(ServiceError::Configuration)?;
        let providers = self
            .config
            .providers
            .declared()
            .filter_map(|(name, runtime)| {
                let factory = self.registry.get(name)?;
                Some(factory.build(&ProviderBuildContext {
                    name,
                    runtime,
                    enabled: self.config.providers.enabled.contains(name),
                    approved: runtime.approved(),
                    credential: credential(runtime),
                    transport: transport.clone(),
                }))
            })
            .collect::<Vec<Arc<dyn Provider>>>();
        Ok(providers
            .into_iter()
            .map(|provider| {
                let (status, code) = if !self.config.data_policy.allows_network_egress() {
                    (
                        ProviderSurfaceStatus::Unavailable,
                        Some("egress_denied".into()),
                    )
                } else {
                    match provider.availability() {
                        ProviderAvailability::Available => (ProviderSurfaceStatus::Available, None),
                        ProviderAvailability::Unavailable { code, .. } => {
                            (ProviderSurfaceStatus::Unavailable, Some(code))
                        }
                    }
                };
                ProviderSummary {
                    schema_version: SCHEMA_VERSION.into(),
                    name: provider.name().into(),
                    status,
                    code,
                    capabilities: provider.capabilities(),
                }
            })
            .collect())
    }

    /// Build every enabled provider from its governance record and factory.
    fn providers(&self) -> Result<Vec<Arc<dyn Provider>>, ServiceError> {
        if self.mock {
            return Ok(mock_providers());
        }
        if !self.config.data_policy.allows_network_egress() {
            return Ok(vec![]);
        }
        let transport = self.transport.clone().ok_or(ServiceError::Configuration)?;
        let mut providers: Vec<Arc<dyn Provider>> = Vec::new();
        for name in &self.config.providers.enabled {
            let runtime = self
                .config
                .providers
                .get(name)
                .ok_or_else(|| ServiceError::ProviderNotDeclared(name.clone()))?;
            let factory = self
                .registry
                .get(name)
                .ok_or_else(|| ServiceError::ProviderNotRegistered(name.clone()))?;
            let provider = factory.build(&ProviderBuildContext {
                name,
                runtime,
                enabled: true,
                approved: runtime.approved(),
                credential: credential(runtime),
                transport: transport.clone(),
            });
            providers.push(self.with_cache(provider, runtime));
        }
        Ok(providers)
    }

    fn with_cache(
        &self,
        provider: Arc<dyn Provider>,
        runtime: &ProviderRuntimeConfig,
    ) -> Arc<dyn Provider> {
        let Some(storage) = self.storage.clone() else {
            return provider;
        };
        Arc::new(CachedProvider::new(
            provider,
            ProviderSearchCache::new(
                storage,
                ProviderSearchCachePolicy {
                    enabled: self.config.cache.provider_search.enabled,
                    ttl_seconds: self.config.cache.provider_search.ttl_seconds,
                    max_entries: self.config.cache.provider_search.max_entries,
                    max_bytes: self.config.cache.provider_search.max_bytes,
                },
            )
            .with_counters(self.cache_counters.clone()),
            runtime
                .adapter_version
                .clone()
                .unwrap_or_else(|| "unknown".into()),
            runtime.storage_rights,
        ))
    }
}

fn storage_failure_message(error: &StorageError) -> String {
    match error {
        StorageError::Corrupt { quarantine_path } => format!(
            "SQLite database was quarantined at {}; persistent cache and telemetry are disabled",
            quarantine_path.display()
        ),
        StorageError::IncompatibleVersion {
            db_version,
            code_version,
        } => format!(
            "database version {db_version} is newer than code version {code_version}; \
             downgrade is not supported, persistent cache and telemetry are disabled"
        ),
        _ => "SQLite persistence could not be opened; persistent cache and telemetry are disabled"
            .into(),
    }
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// Truncate on a character boundary so bounded text never splits UTF-8.
fn bounded(value: &str, maximum: usize) -> String {
    value.chars().take(maximum).collect()
}

fn credential(config: &ProviderRuntimeConfig) -> Option<String> {
    config
        .credential_env
        .as_deref()
        .and_then(|name| std::env::var(name).ok())
        .filter(|value| !value.is_empty())
}

/// Fail-closed canary preflight against the providers AMATL ships.
pub fn validate_provider_canary(
    config: &Config,
    provider: &str,
) -> Result<(), ProviderCanaryError> {
    validate_provider_canary_with(config, &ProviderRegistry::builtin(), provider)
}

/// Fail-closed canary preflight against a caller-supplied registry.
///
/// Checks run before any network access: policy, enablement, registration,
/// canary support, governance approval and credential presence.
pub fn validate_provider_canary_with(
    config: &Config,
    registry: &ProviderRegistry,
    provider: &str,
) -> Result<(), ProviderCanaryError> {
    if !config.data_policy.allows_network_egress() {
        return Err(ProviderCanaryError::EgressDenied);
    }
    if !config.providers.enabled.iter().any(|name| name == provider) {
        return Err(ProviderCanaryError::NotEnabled(provider.into()));
    }
    let runtime = config
        .providers
        .get(provider)
        .ok_or_else(|| ProviderCanaryError::UnknownProvider(provider.into()))?;
    let factory = registry
        .get(provider)
        .ok_or_else(|| ProviderCanaryError::UnknownProvider(provider.into()))?;
    if !factory.supports_network_canary() {
        return Err(ProviderCanaryError::NetworkBlocked(provider.into()));
    }
    if !runtime.approved() {
        return Err(ProviderCanaryError::Governance(provider.into()));
    }
    if !factory.requires_credential() {
        return Ok(());
    }
    let credential_name = runtime
        .credential_env
        .as_deref()
        .filter(|name| !name.trim().is_empty())
        .ok_or_else(|| ProviderCanaryError::Credential(provider.into()))?;
    if std::env::var(credential_name)
        .ok()
        .is_none_or(|value| value.trim().is_empty())
    {
        return Err(ProviderCanaryError::Credential(provider.into()));
    }
    Ok(())
}

struct DeniedFetcher;

#[async_trait]
impl crate::Fetcher for DeniedFetcher {
    async fn fetch(&self, _: crate::FetchRequest) -> Result<crate::FetchResult, crate::FetchError> {
        tracing::warn!(
            target: "amatl::security",
            security_event = "egress_denied",
            operation = "public_fetch",
            "Data policy denied outbound network access"
        );
        Err(crate::FetchError::EgressDenied)
    }
}

struct DeniedHttpTransport;

#[async_trait]
impl crate::HttpTransport for DeniedHttpTransport {
    async fn execute(&self, _: crate::HttpRequest) -> Result<crate::HttpResponse, String> {
        tracing::warn!(
            target: "amatl::security",
            security_event = "egress_denied",
            operation = "provider_request",
            "Data policy denied outbound network access"
        );
        Err("network egress denied by data policy".into())
    }
}

fn mock_providers() -> Vec<Arc<dyn Provider>> {
    let shared = ProviderItem {
        title: Some("Rust async programming guide".into()),
        url: "https://example.com/rust?utm_source=mock".into(),
        provider_rank: Some(Rank::FIRST),
        snippet: Some("A deterministic mock result".into()),
        result_type: None,
        published_at: None,
        author: None,
        language: None,
        file_type: None,
        thumbnail: None,
        metadata: Default::default(),
    };
    vec![
        Arc::new(MockProvider::success("mock-a", vec![shared.clone()])),
        Arc::new(MockProvider::success("mock-b", vec![shared])),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ProviderFactory;

    struct ArchiveFactory;

    impl ProviderFactory for ArchiveFactory {
        fn name(&self) -> &str {
            "custom_archive"
        }

        fn requires_credential(&self) -> bool {
            false
        }

        fn build(&self, context: &ProviderBuildContext<'_>) -> Arc<dyn Provider> {
            Arc::new(MockProvider::success(
                context.name,
                vec![ProviderItem {
                    title: Some("Archived rust guide".into()),
                    url: "https://archive.invalid/rust".into(),
                    provider_rank: Some(Rank::FIRST),
                    snippet: Some("A registered third-party source".into()),
                    result_type: None,
                    published_at: None,
                    author: None,
                    language: None,
                    file_type: None,
                    thumbnail: None,
                    metadata: Default::default(),
                }],
            ))
        }
    }

    fn archive_config() -> Config {
        let mut config = Config::default();
        config.providers.declare(
            "custom_archive",
            ProviderRuntimeConfig {
                adapter_version: Some("archive-v1".into()),
                ..ProviderRuntimeConfig::default()
            },
        );
        config.providers.enabled = vec!["custom_archive".into()];
        config
    }

    fn isolated_config() -> Config {
        let mut config = Config::default();
        config.data_policy.profile = crate::SecurityProfile::Isolated;
        config.data_policy.egress = crate::EgressPolicy::Deny;
        config.data_policy.inference = crate::InferenceMode::LocalOnly;
        config
    }

    #[test]
    fn mcp_default_limits_are_strictly_below_cli_for_expensive_work() {
        let config = Config::default();
        let cli = ExecutionLimits::for_surface(&config, ServiceSurface::cli());
        let mcp = ExecutionLimits::for_surface(&config, ServiceSurface::mcp());
        assert!(mcp.max_provider_calls <= cli.max_provider_calls);
        assert!(mcp.search_timeout_ms < cli.search_timeout_ms);
        assert!(mcp.deep_max_fetches < cli.deep_max_fetches);
        assert!(mcp.deep_max_bytes < cli.deep_max_bytes);
        assert!(mcp.deep_max_subqueries < cli.deep_max_subqueries);
    }

    #[tokio::test]
    async fn shared_service_preserves_the_public_search_contract() {
        let service = AmatlService::new(Config::default(), true).await;
        let result = service
            .search("rust".into(), ServiceSurface::api(None))
            .await
            .unwrap();
        assert_eq!(result.response.schema_version, SCHEMA_VERSION);
        assert_eq!(result.response.results.len(), 1);
    }

    #[tokio::test]
    async fn telemetry_lives_for_the_lifetime_of_the_service() {
        let service = AmatlService::new(Config::default(), true).await;
        service
            .search("first".into(), ServiceSurface::api(None))
            .await
            .unwrap();
        let first = service.telemetry.status().in_memory_observations;
        assert!(first > 0);
        service
            .search("second".into(), ServiceSurface::api(None))
            .await
            .unwrap();
        assert!(service.telemetry.status().in_memory_observations > first);
    }

    #[tokio::test]
    async fn isolated_service_denies_public_fetch_without_network_access() {
        let service = AmatlService::new(isolated_config(), false).await;
        let result = service
            .fetch_public(crate::FetchRequest {
                url: url::Url::parse("https://example.com/private?token=never-send").unwrap(),
                timeout_ms: 1_000,
                max_bytes: 1_024,
                max_redirects: 0,
                headers: Default::default(),
                request_id: None,
            })
            .await;
        assert!(matches!(result, Err(crate::FetchError::EgressDenied)));
    }

    #[tokio::test]
    async fn isolated_deep_degrades_instead_of_fetching_candidates() {
        let service = AmatlService::new(isolated_config(), true).await;
        let response = service
            .deep("rust".into(), ServiceSurface::api(None))
            .await
            .unwrap();
        assert!(response.documents.is_empty());
        assert!(response
            .degradations
            .iter()
            .any(|degradation| degradation.code == "egress_denied"));
    }

    #[tokio::test]
    async fn isolated_provider_surface_reports_the_central_policy() {
        let service = AmatlService::new(isolated_config(), false).await;
        let summaries = service.provider_summaries().unwrap();
        assert!(!summaries.is_empty());
        assert!(summaries.iter().all(|summary| {
            summary.status == ProviderSurfaceStatus::Unavailable
                && summary.code.as_deref() == Some("egress_denied")
        }));
        assert_eq!(
            validate_provider_canary(service.config(), "brave"),
            Err(ProviderCanaryError::EgressDenied)
        );
    }

    #[test]
    fn real_provider_canary_is_fail_closed_before_network_access() {
        let config = Config::default();
        assert_eq!(
            validate_provider_canary(&config, "brave"),
            Err(ProviderCanaryError::NotEnabled("brave".into()))
        );
        let mut enabled = config;
        enabled.providers.enabled = vec!["brave".into()];
        assert_eq!(
            validate_provider_canary(&enabled, "brave"),
            Err(ProviderCanaryError::Governance("brave".into()))
        );
        enabled.providers.enabled = vec!["duckduckgo_html".into()];
        assert_eq!(
            validate_provider_canary(&enabled, "duckduckgo_html"),
            Err(ProviderCanaryError::NetworkBlocked(
                "duckduckgo_html".into()
            ))
        );
    }

    #[tokio::test]
    async fn a_registered_third_party_source_runs_without_core_changes() {
        let config = archive_config();
        assert!(config.validate().is_ok());
        let service = AmatlService::with_registry(
            config,
            false,
            ProviderRegistry::builtin().with(Arc::new(ArchiveFactory)),
        )
        .await;
        let execution = service
            .search("rust".into(), ServiceSurface::cli())
            .await
            .unwrap();
        assert_eq!(
            execution.response.providers_used,
            vec!["custom_archive".to_string()]
        );
        assert_eq!(execution.response.results.len(), 1);
        let summaries = service.provider_summaries().unwrap();
        assert!(summaries
            .iter()
            .any(|summary| summary.name == "custom_archive"));
        assert!(summaries.iter().any(|summary| summary.name == "brave"));
    }

    #[tokio::test]
    async fn a_declared_provider_without_implementation_reports_a_precise_code() {
        let service =
            AmatlService::with_registry(archive_config(), false, ProviderRegistry::builtin()).await;
        let error = service
            .search("rust".into(), ServiceSurface::cli())
            .await
            .unwrap_err();
        assert_eq!(
            error,
            ServiceError::ProviderNotRegistered("custom_archive".into())
        );
        assert_eq!(error.code(), crate::ErrorCode::ProviderNotRegistered);
    }

    #[test]
    fn canary_preflight_uses_the_supplied_registry() {
        let mut config = archive_config();
        assert_eq!(
            validate_provider_canary_with(&config, &ProviderRegistry::builtin(), "custom_archive"),
            Err(ProviderCanaryError::UnknownProvider(
                "custom_archive".into()
            ))
        );
        let registry = ProviderRegistry::builtin().with(Arc::new(ArchiveFactory));
        assert_eq!(
            validate_provider_canary_with(&config, &registry, "custom_archive"),
            Err(ProviderCanaryError::Governance("custom_archive".into()))
        );
        let runtime = config.providers.entry("custom_archive");
        runtime.approval_status = crate::ApprovalStatus::Approved;
        runtime.reviewed_at = Some(today_iso());
        runtime.reviewer = Some("owner".into());
        runtime.terms_url = Some("https://archive.invalid/terms".into());
        runtime.terms_version_or_date = Some("2026-08-01".into());
        runtime.allowed_access_method = Some("official_api".into());
        runtime.plan_or_contract = Some("contract-1".into());
        runtime.rate_limit = Some("1 qps".into());
        runtime.cost_model = Some("free".into());
        runtime.data_handling_notes = Some("no cache".into());
        runtime.operational_risk = Some("none".into());
        // The factory declares no credential requirement, so approval is enough.
        assert_eq!(
            validate_provider_canary_with(&config, &registry, "custom_archive"),
            Ok(())
        );
    }

    fn today_iso() -> String {
        let now = time::OffsetDateTime::now_utc().date();
        format!(
            "{:04}-{:02}-{:02}",
            now.year(),
            u8::from(now.month()),
            now.day()
        )
    }

    fn semantic_config() -> Config {
        let mut config = Config::default();
        config.deep.ranking_v2.policy.weight_bm25 = 0.6;
        config.deep.ranking_v2.policy.weight_semantic = 0.3;
        config.deep.ranking_v2.policy.weight_reranker = 0.1;
        config
    }

    #[tokio::test]
    async fn local_inference_backs_the_semantic_ranking_weights() {
        let mut config = semantic_config();
        config.data_policy.inference = crate::InferenceMode::LocalOnly;
        assert!(config.validate().is_ok());
        let service = AmatlService::new(config, true).await;
        assert_eq!(
            service.inference_backend(),
            Some(crate::LOCAL_EMBEDDING_BACKEND_ID)
        );
        let response = service
            .deep("rust".into(), ServiceSurface::cli())
            .await
            .unwrap();
        assert!(
            !response
                .degradations
                .iter()
                .any(|degradation| degradation.code
                    == crate::ErrorCode::InferenceUnavailable.as_str())
        );
    }

    #[tokio::test]
    async fn missing_inference_backend_degrades_instead_of_faking_semantic_ranking() {
        // Configuration validation rejects this pairing; the service must still
        // fail safe if it is constructed programmatically.
        let service = AmatlService::new(semantic_config(), true).await;
        assert_eq!(service.inference_backend(), None);
        let response = service
            .deep("rust".into(), ServiceSurface::cli())
            .await
            .unwrap();
        assert!(
            response
                .degradations
                .iter()
                .any(|degradation| degradation.code
                    == crate::ErrorCode::InferenceUnavailable.as_str())
        );
    }

    #[tokio::test]
    async fn remote_inference_never_silently_falls_back_to_a_local_backend() {
        let mut config = Config::default();
        config.data_policy.inference = crate::InferenceMode::RemoteExplicit;
        let service = AmatlService::new(config, true).await;
        assert_eq!(service.inference_backend(), None);
    }
}
