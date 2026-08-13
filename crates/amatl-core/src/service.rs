use crate::{
    parse_query, BraveProvider, Budget, CachedProvider, ChromiumRenderer, Config, DeepBudget,
    DeepCandidate, DeepOrchestrator, DeepRequest, DeepResponse, DocumentCache, DocumentCachePolicy,
    DuckDuckGoHtmlProvider, GapAnalyzer, InMemoryTelemetry, MockProvider, MojeekProvider, Provider,
    ProviderAvailability, ProviderCapabilities, ProviderItem, ProviderRuntimeConfig,
    ProviderSearchCache, ProviderSearchCachePolicy, Query, Rank, RankingV2Engine, ReqwestTransport,
    SafeFetcher, SearchOrchestrator, SearchPlan, SearchResponse, SearchSubQueryExecutor,
    SqliteStorage, TrafilaturaExtractor, SCHEMA_VERSION,
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

#[derive(Clone)]
pub struct AmatlService {
    config: Arc<Config>,
    storage: Option<SqliteStorage>,
    mock: bool,
}

impl AmatlService {
    pub async fn new(config: Config, mock: bool) -> Self {
        let storage = if config.persistence.enabled {
            SqliteStorage::open(&config.persistence.path).await.ok()
        } else {
            None
        };
        Self {
            config: Arc::new(config),
            storage,
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
        let telemetry = InMemoryTelemetry::with_optional_storage(
            self.config
                .telemetry
                .persistence_enabled
                .then_some(self.storage.clone())
                .flatten(),
        )
        .await;
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
        .with_telemetry(telemetry);
        let response = orchestrator.search(query.clone(), providers).await;
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
            Arc::new(SafeFetcher::default()),
            extractor,
            renderer,
            document_cache,
            limits.deep_timeout_ms,
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
        Ok(orchestrator
            .enrich(DeepRequest {
                query: search.query,
                search_plan: search.plan,
                candidates,
            })
            .await)
    }

    pub fn provider_summaries(&self) -> Result<Vec<ProviderSummary>, ServiceError> {
        let transport = Arc::new(
            ReqwestTransport::new(2 * 1024 * 1024).map_err(|_| ServiceError::Configuration)?,
        );
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
        let transport = Arc::new(
            ReqwestTransport::new(2 * 1024 * 1024).map_err(|_| ServiceError::Configuration)?,
        );
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
                _ => {}
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

fn credential(config: &ProviderRuntimeConfig) -> Option<String> {
    config
        .credential_env
        .as_deref()
        .and_then(|name| std::env::var(name).ok())
        .filter(|value| !value.is_empty())
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
}
