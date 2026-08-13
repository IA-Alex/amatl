use crate::{
    parse_query, BraveProvider, Budget, CachedProvider, ChromiumRenderer, Config, DeepBudget,
    DeepCandidate, DeepOrchestrator, DeepRequest, DeepResponse, DocumentCache, DocumentCachePolicy,
    DuckDuckGoHtmlProvider, GapAnalyzer, InMemoryTelemetry, MockProvider, MojeekProvider, Provider,
    ProviderAvailability, ProviderCapabilities, ProviderItem, ProviderRuntimeConfig,
    ProviderSearchCache, ProviderSearchCachePolicy, Query, Rank, RankingV2Engine, ReqwestTransport,
    SafeFetcher, SearchOrchestrator, SearchPlan, SearchResponse, SearchSubQueryExecutor,
    SqliteStorage, StorageError, TrafilaturaExtractor, SCHEMA_VERSION,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use thiserror::Error;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServiceSurface {
    Cli,
    Api,
    Mcp,
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
        match surface {
            ServiceSurface::Cli | ServiceSurface::Api => base,
            ServiceSurface::Mcp => Self {
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

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum ServiceError {
    #[error("invalid query")]
    InvalidQuery,
    #[error("search planning failed")]
    MissingPlan,
    #[error("service configuration failed")]
    Configuration,
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum ProviderCanaryError {
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

#[derive(Clone)]
pub struct AmatlService {
    config: Arc<Config>,
    storage: Option<SqliteStorage>,
    storage_degradation: Option<crate::Degradation>,
    telemetry: InMemoryTelemetry,
    transport: Option<Arc<dyn crate::HttpTransport>>,
    fetcher: Arc<SafeFetcher>,
    mock: bool,
}

impl AmatlService {
    pub async fn new(config: Config, mock: bool) -> Self {
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
                            code: "storage_unavailable".into(),
                            component: "sqlite".into(),
                            message,
                        }),
                    )
                }
            }
        } else {
            (None, None)
        };
        let telemetry = InMemoryTelemetry::with_optional_storage(
            config
                .telemetry
                .persistence_enabled
                .then(|| storage.clone())
                .flatten(),
        )
        .await;
        let transport = ReqwestTransport::new(2 * 1024 * 1024)
            .map(|transport| Arc::new(transport) as Arc<dyn crate::HttpTransport>)
            .map_err(|error| {
                tracing::warn!(
                    target: "amatl::providers",
                    error = %error,
                    "provider HTTP transport could not be initialized"
                );
                error
            })
            .ok();
        Self {
            config: Arc::new(config),
            storage,
            storage_degradation,
            telemetry,
            transport,
            fetcher: Arc::new(SafeFetcher::default()),
            mock,
        }
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    pub fn storage(&self) -> Option<SqliteStorage> {
        self.storage.clone()
    }

    pub async fn search(
        &self,
        raw_query: String,
        surface: ServiceSurface,
    ) -> Result<SearchExecution, ServiceError> {
        let query = parse_query(raw_query).map_err(|_| ServiceError::InvalidQuery)?;
        let limits = ExecutionLimits::for_surface(&self.config, surface);
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
        .with_telemetry(self.telemetry.clone());
        let mut response = orchestrator.search(query.clone(), providers).await;
        if let Some(degradation) = &self.storage_degradation {
            response.degradations.push(degradation.clone());
            if response.status == crate::SearchStatus::Success {
                response.status = crate::SearchStatus::PartialSuccess;
            }
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
        let search = self.search(raw_query, surface).await?;
        let limits = ExecutionLimits::for_surface(&self.config, surface);
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
                    },
                )
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
                        .all(|provider| match provider.as_str() {
                            "brave" => self.config.providers.brave.storage_rights,
                            "mojeek" => self.config.providers.mojeek.storage_rights,
                            _ => false,
                        });
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
        let renderer = Arc::new(ChromiumRenderer::detect(&self.config.deep.renderer));
        let mut orchestrator = DeepOrchestrator::new(
            deep_budget,
            self.fetcher.clone(),
            extractor,
            renderer,
            document_cache,
            remaining_deep_ms,
            (limits.deep_max_bytes / u64::from(limits.deep_max_fetches)).max(1),
            self.config.deep.max_redirects,
            self.config.deep.top_k as usize,
            self.config.deep.max_depth,
        );
        if self.config.deep.ranking_v2.enabled {
            orchestrator = orchestrator.with_ranking_v2(
                RankingV2Engine::new(self.config.deep.ranking_v2.policy.clone())
                    .map_err(|_| ServiceError::Configuration)?,
            );
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
        if let Some(degradation) = &self.storage_degradation {
            response.degradations.push(degradation.clone());
        }
        Ok(response)
    }

    pub fn provider_summaries(&self) -> Result<Vec<ProviderSummary>, ServiceError> {
        let transport = self.transport.clone().ok_or(ServiceError::Configuration)?;
        let providers: Vec<Arc<dyn Provider>> = vec![
            Arc::new(BraveProvider::new(
                credential(&self.config.providers.brave),
                self.config.providers.enabled.contains(&"brave".to_string()),
                self.config.providers.brave.approved(),
                transport.clone(),
            )),
            Arc::new(MojeekProvider::new(
                credential(&self.config.providers.mojeek),
                self.config
                    .providers
                    .enabled
                    .contains(&"mojeek".to_string()),
                self.config.providers.mojeek.approved(),
                self.config.providers.mojeek.supported_filters.clone(),
                transport,
            )),
            Arc::new(DuckDuckGoHtmlProvider::blocked()),
        ];
        Ok(providers
            .into_iter()
            .map(|provider| {
                let (status, code) = match provider.availability() {
                    ProviderAvailability::Available => (ProviderSurfaceStatus::Available, None),
                    ProviderAvailability::Unavailable { code, .. } => {
                        (ProviderSurfaceStatus::Unavailable, Some(code))
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

    fn providers(&self) -> Result<Vec<Arc<dyn Provider>>, ServiceError> {
        if self.mock {
            return Ok(mock_providers());
        }
        let transport = self.transport.clone().ok_or(ServiceError::Configuration)?;
        let mut providers: Vec<Arc<dyn Provider>> = Vec::new();
        for name in &self.config.providers.enabled {
            match name.as_str() {
                "brave" => providers.push(self.with_cache(
                    Arc::new(BraveProvider::new(
                        credential(&self.config.providers.brave),
                        true,
                        self.config.providers.brave.approved(),
                        transport.clone(),
                    )),
                    &self.config.providers.brave,
                )),
                "mojeek" => providers.push(self.with_cache(
                    Arc::new(MojeekProvider::new(
                        credential(&self.config.providers.mojeek),
                        true,
                        self.config.providers.mojeek.approved(),
                        self.config.providers.mojeek.supported_filters.clone(),
                        transport.clone(),
                    )),
                    &self.config.providers.mojeek,
                )),
                "duckduckgo_html" => providers.push(Arc::new(DuckDuckGoHtmlProvider::blocked())),
                _ => return Err(ServiceError::Configuration),
            }
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
            ),
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
        _ => "SQLite persistence could not be opened; persistent cache and telemetry are disabled"
            .into(),
    }
}

fn credential(config: &ProviderRuntimeConfig) -> Option<String> {
    config
        .credential_env
        .as_deref()
        .and_then(|name| std::env::var(name).ok())
        .filter(|value| !value.is_empty())
}

pub fn validate_provider_canary(
    config: &Config,
    provider: &str,
) -> Result<(), ProviderCanaryError> {
    if !config.providers.enabled.iter().any(|name| name == provider) {
        return Err(ProviderCanaryError::NotEnabled(provider.into()));
    }
    let runtime = match provider {
        "brave" => &config.providers.brave,
        "mojeek" => &config.providers.mojeek,
        "duckduckgo_html" => {
            return Err(ProviderCanaryError::NetworkBlocked(provider.into()));
        }
        _ => return Err(ProviderCanaryError::UnknownProvider(provider.into())),
    };
    if !runtime.approved() {
        return Err(ProviderCanaryError::Governance(provider.into()));
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

    #[test]
    fn mcp_default_limits_are_strictly_below_cli_for_expensive_work() {
        let config = Config::default();
        let cli = ExecutionLimits::for_surface(&config, ServiceSurface::Cli);
        let mcp = ExecutionLimits::for_surface(&config, ServiceSurface::Mcp);
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
            .search("rust".into(), ServiceSurface::Api)
            .await
            .unwrap();
        assert_eq!(result.response.schema_version, SCHEMA_VERSION);
        assert_eq!(result.response.results.len(), 1);
    }

    #[tokio::test]
    async fn telemetry_lives_for_the_lifetime_of_the_service() {
        let service = AmatlService::new(Config::default(), true).await;
        service
            .search("first".into(), ServiceSurface::Api)
            .await
            .unwrap();
        let first = service.telemetry.status().in_memory_observations;
        assert!(first > 0);
        service
            .search("second".into(), ServiceSurface::Api)
            .await
            .unwrap();
        assert!(service.telemetry.status().in_memory_observations > first);
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
}
