use crate::audit::SecurityAudit;
use crate::cache::{CacheCounters, CacheEffectiveness};
use crate::circuit::{CircuitSnapshot, ProviderCircuit};
use crate::storage::{CacheStats, SavedDocument, SearchHistoryEntry};
use crate::telemetry::{now_unix, ProviderValueSnapshot};
use crate::{
    parse_query, Answer, AnswerError, Budget, CachedProvider, CanonicalUrl, ChromiumRenderer,
    CompletionBackend, Config, DeepBudget, DeepCandidate, DeepOrchestrator, DeepRequest,
    DeepResponse, DocumentCache, DocumentCachePolicy, ErrorCode, FallbackExtractor, GapAnalyzer,
    InMemoryTelemetry, InferenceRuntime, MockProvider, NativeHtmlExtractor, Provider,
    ProviderAvailability, ProviderBuildContext, ProviderCapabilities, ProviderItem,
    ProviderRegistry, ProviderRuntimeConfig, ProviderSearchCache, ProviderSearchCachePolicy, Query,
    Rank, RankingV2Engine, RemoteCompletionBackend, RendererPool, ReqwestTransport, RoleAssignment,
    SafeFetcher, SearchOrchestrator, SearchPlan, SearchResponse, SearchResult,
    SearchSubQueryExecutor, SqliteStorage, StorageError, TrafilaturaExtractor, SCHEMA_VERSION,
};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, RwLock};
use thiserror::Error;
use zeroize::Zeroizing;

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
    /// Deadline for one single-URL fetch exposed to a surface.
    pub fetch_timeout_ms: u64,
    /// Byte ceiling for one single-URL fetch exposed to a surface.
    pub fetch_max_bytes: u64,
    /// Redirect ceiling for one single-URL fetch exposed to a surface.
    pub fetch_max_redirects: u32,
    /// Largest page a surface may request in one search.
    pub max_page_size: u32,
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
            // One fetch never gets the whole Deep budget: it is bounded by the
            // per-fetch share of the configured deep limits.
            fetch_timeout_ms: config.deep.extractor.timeout_ms.min(config.deep.timeout_ms),
            fetch_max_bytes: (config.deep.max_bytes / u64::from(config.deep.max_fetches.max(1)))
                .max(1),
            fetch_max_redirects: config.deep.max_redirects,
            max_page_size: 100,
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
                fetch_timeout_ms: base.fetch_timeout_ms.min(3_000),
                fetch_max_bytes: base.fetch_max_bytes.min(256 * 1024),
                fetch_max_redirects: base.fetch_max_redirects.min(2),
                max_page_size: base.max_page_size.min(25),
            },
        }
    }
}

/// Providers admitted into a round plus the reasons for the refusals.
#[derive(Default)]
struct ProviderSelection {
    providers: Vec<Arc<dyn Provider>>,
    degradations: Vec<crate::Degradation>,
}

#[derive(Clone, Debug)]
pub struct SearchExecution {
    pub query: Query,
    pub plan: SearchPlan,
    pub response: SearchResponse,
}

/// One query's outcome in a `search_batch` call.
///
/// Items are independent: a query that fails validation or execution yields an
/// `error` while the rest of the batch still completes, so an orchestrator can
/// consume the usable rows and treat the failed one separately instead of the
/// whole batch collapsing. `response` and `error` are mutually exclusive.
#[derive(Clone, Debug, Serialize)]
pub struct SearchBatchItem {
    pub query: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response: Option<SearchResponse>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorCode>,
}

/// One Search result explicitly selected by a Deep caller.
///
/// This deliberately accepts only identity and provenance already exposed by
/// `SearchResult`; Deep always performs the network acquisition itself.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DeepTarget {
    pub url: String,
    #[serde(default)]
    pub title: Option<String>,
    pub provider: String,
}

/// Output of [`AmatlService::answer`]: the search AMATL ran plus the grounded
/// synthesis on top of it, kept separate on purpose so a caller always has
/// the raw results even when it only wants the summary.
#[derive(Clone, Debug, Serialize)]
pub struct AnswerResult {
    pub schema_version: String,
    pub answer: Answer,
    pub search: SearchResponse,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProviderSurfaceStatus {
    Available,
    Unavailable,
}

/// Where a source's credential comes from, if any.
///
/// Exposed so an operator surface can tell an in-memory key (liquidable from
/// the UI, never persisted) apart from one supplied by the environment variable
/// named in `credential_env` (which the UI cannot erase). It never carries the
/// value itself, only its provenance.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CredentialSource {
    /// No credential is available for this source.
    None,
    /// An operator injected the key in memory; it is liquidable at runtime and
    /// dies with the process.
    Ephemeral,
    /// The key was read from the environment variable named by `credential_env`.
    Environment,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderSummary {
    pub schema_version: String,
    pub name: String,
    pub status: ProviderSurfaceStatus,
    pub code: Option<String>,
    pub capabilities: ProviderCapabilities,
    /// Whether `name` is currently in `providers.enabled` — independent of
    /// `status`: an enabled provider can still be `Unavailable` (missing
    /// approval, credential, or egress), and a disabled one is always
    /// `Unavailable` regardless of how complete its ficha is. Surfaces
    /// exist to flip this without also claiming they approved anything.
    pub enabled: bool,
    /// Provenance of the credential in force, never its value. Lets a settings
    /// screen enable "liquidate" only for a key it can actually erase.
    pub credential_source: CredentialSource,
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
    #[error("answer synthesis backend is unavailable or its response was not grounded")]
    AnswerUnavailable,
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
            Self::AnswerUnavailable => ErrorCode::AnswerUnavailable,
        }
    }
}

fn validate_deep_fetch_cap(
    requested: Option<u32>,
    configured_maximum: u32,
) -> Result<(), ServiceError> {
    if requested.is_some_and(|value| value == 0 || value > configured_maximum) {
        Err(ServiceError::InvalidInput)
    } else {
        Ok(())
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
    /// Read-only view of the answer-synthesis configuration: never the
    /// credential, only what a settings screen needs to explain *why* the
    /// feature is or isn't available right now.
    pub answer: AnswerStatus,
    /// Read-only view of the search-strategist configuration: whether the LLM
    /// is allowed to participate in search (refining the query between
    /// rounds). Never the credential — only what a settings screen needs to
    /// explain why the "search with model" toggle is or isn't usable.
    pub strategist: StrategistStatus,
    /// Read-only view of `data_policy` — the profile, egress and inference
    /// gate every other capability in this status checks against. Exposed
    /// so a settings screen can explain *why* a source, remote embedding or
    /// `answer` call is unavailable instead of just reporting that it is.
    pub data_policy: DataPolicyStatus,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DataPolicyStatus {
    pub profile: crate::config::SecurityProfile,
    pub egress: crate::config::EgressPolicy,
    pub inference: crate::config::InferenceMode,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct AnswerStatus {
    /// `answer.enabled` in configuration — the operator's on/off intent,
    /// exactly what the web toggle flips. Independent of `configured`: an
    /// operator can turn this off without erasing `endpoint`/`model`, and a
    /// settings screen needs to keep showing those to make the toggle mean
    /// anything when it's off.
    pub enabled: bool,
    /// Whether `endpoint` and `model` are both present, regardless of
    /// `enabled` — "is there a real configuration on disk to toggle",
    /// answered even while it's switched off.
    pub configured: bool,
    /// Whether a call to `answer()` will actually work right now: `enabled`
    /// AND `configured` AND the credential was present in the environment
    /// at startup. A UI should gate the feature on this, not on `enabled` or
    /// `configured` alone.
    pub available: bool,
    /// Shown whenever set on disk, independent of `enabled` — see the field
    /// doc on `enabled` for why.
    pub model: Option<String>,
    pub endpoint: Option<String>,
}

/// Read-only status of the search strategist — the optional LLM that, when
/// enabled, participates in a search by proposing a refined query that AMATL
/// then runs to complement the first result set (see `service.rs`:
/// `search_inner` and the `[strategist]` config section).
///
/// Mirrors [`AnswerStatus`]: it reports the operator's on/off intent, whether
/// there is a real model/endpoint configured, and whether a call would
/// actually work right now (enabled + configured + credential present). It
/// never exposes the credential.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct StrategistStatus {
    /// `strategist.enabled` in configuration — exactly what the web toggle
    /// flips. Independent of `configured`: an operator can turn it off
    /// without erasing `endpoint`/`model`, and the settings screen needs to
    /// keep showing those for the toggle to mean anything when it's off.
    pub enabled: bool,
    /// Whether `endpoint` and `model` are both present, regardless of
    /// `enabled` — "is there a real configuration on disk to toggle".
    pub configured: bool,
    /// Whether calling the strategist would work right now: `enabled` AND
    /// `configured` AND the credential environment variable was present.
    pub available: bool,
    /// Model identifier, shown whenever set on disk, independent of
    /// `enabled`.
    pub model: Option<String>,
    /// Endpoint, shown whenever set on disk, independent of `enabled`.
    pub endpoint: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SourceStatus {
    pub name: String,
    pub status: ProviderSurfaceStatus,
    pub code: Option<String>,
    /// Breaker state: `closed` when the source may be called, `open` while it
    /// is in cooldown, `half_open` for the single probe after a cooldown.
    pub circuit: crate::circuit::CircuitState,
    /// Observed success ratio in the telemetry window, when there are samples.
    pub success_rate: Option<f64>,
    pub average_latency_ms: Option<f64>,
    pub observations: u64,
    /// Provenance of the credential in force for this source, never its value.
    pub credential_source: CredentialSource,
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
    /// Governed answer-synthesis backend; `None` whenever `answer.enabled` is
    /// false or its configuration/credential is unavailable, in which case
    /// [`AmatlService::answer`] fails closed with `AnswerUnavailable` instead
    /// of falling back to anything.
    answer_backend: Option<Arc<dyn CompletionBackend>>,
    mock: bool,
    /// Renderer pool created once at startup and reused across deep requests.
    renderer_pool: RendererPool,
    /// Shared cache hit/miss counters for the operator status surface.
    cache_counters: Arc<CacheCounters>,
    /// Persistent breaker that removes a currently failing source from a round.
    circuit: ProviderCircuit,
    /// Durable security audit trail; inert without persistence.
    audit: SecurityAudit,
    strategist: Arc<dyn crate::strategist::SearchStrategist + Send + Sync>,
    /// Parsed operator Optics document (`[ranking.optics]`). Loaded and parsed
    /// exactly once here -- on first build and again on every `reloaded()`,
    /// which rebuilds the whole service -- never per search. `None` when
    /// Optics is disabled, unconfigured, or its file was missing/unparsable
    /// (a warning is logged in the last case; a search never fails for it).
    optics: Option<Arc<crate::optics::OpticsDocument>>,
    /// Operator-supplied provider credentials kept only in RAM (never on disk,
    /// never in `amatl.toml`, never in logs). Keyed by provider name and
    /// consulted before the `credential_env` environment variable, so an admin
    /// can insert/liquidate a source's key at runtime (e.g. Marginalia) without
    /// touching the environment or restarting. Killed with the process.
    /// `Arc<RwLock>` so changes take effect for subsequent requests without a
    /// service rebuild, and survive `reloaded()` by being shared in. Values are
    /// `Zeroizing` so replacing or liquidating a key wipes its bytes from the
    /// heap instead of leaving them for the allocator to reuse.
    ephemeral_credentials: Arc<RwLock<BTreeMap<String, Zeroizing<String>>>>,
    /// Stops the background maintenance task when the service is dropped.
    ///
    /// Must be a `DropGuard`, not a bare `CancellationToken`: a token does not
    /// cancel on drop, so the task would outlive the service and keep the
    /// connection pool and the advisory file lock alive forever. `Arc` because
    /// the service is `Clone`; the task stops when the last clone is dropped.
    _maintenance_cancel: Option<Arc<tokio_util::sync::DropGuard>>,
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
            match SqliteStorage::open(
                std::path::PathBuf::from(&config.persistence.path),
                config.persistence.locking_mode,
            )
            .await
            {
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
        // Spawn background maintenance if persistence is enabled and either
        // purging or automatic backups are configured.
        let maintenance_wanted =
            config.persistence.purge_interval_seconds > 0 || config.persistence.auto_backup_enabled;
        let maintenance_cancel = match (&storage, maintenance_wanted) {
            (Some(storage), true) => {
                let storage_arc = Arc::new(storage.clone());
                Some(Arc::new(
                    storage_arc
                        .spawn_maintenance(&config.persistence, config.telemetry.retention_days)
                        .drop_guard(),
                ))
            }
            _ => None,
        };

        let config_circuit_policy = config.circuit_breaker;
        let storage_for_circuit = storage.clone();
        let storage_for_audit = storage.clone();
        let audit_retention_days = config.persistence.audit_retention_days;
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
        let inference = match InferenceRuntime::from_policy(
            &config.data_policy,
            &config.inference,
            transport.clone(),
        ) {
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
        let answer_backend: Option<Arc<dyn CompletionBackend>> = if config.answer.enabled {
            match transport.clone() {
                Some(transport) => match RemoteCompletionBackend::new(&config.answer, transport) {
                    Ok(backend) => Some(Arc::new(backend) as Arc<dyn CompletionBackend>),
                    Err(error) => {
                        tracing::warn!(
                            target: "amatl::answer",
                            error = %error,
                            "answer synthesis backend is unavailable; answer() will fail closed"
                        );
                        None
                    }
                },
                None => None,
            }
        } else {
            None
        };
        let renderer = Arc::new(ChromiumRenderer::detect(&config.deep.renderer));
        let renderer_pool =
            RendererPool::new(renderer, config.deep.renderer.max_browser_calls as usize);
        let optics = load_optics(&config.ranking.optics);
        Self {
            config: Arc::new(config.clone()),
            registry: Arc::new(registry),
            storage,
            storage_degradation,
            telemetry,
            transport,
            fetcher,
            inference,
            answer_backend,
            optics,
            mock,
            renderer_pool,
            cache_counters: Arc::new(CacheCounters::default()),
            circuit: ProviderCircuit::restored(config_circuit_policy, storage_for_circuit).await,
            audit: SecurityAudit::new(storage_for_audit, audit_retention_days),
            ephemeral_credentials: Arc::new(RwLock::new(BTreeMap::new())),
            strategist: if config.strategist.enabled {
                match crate::strategist::llm::LlmStrategist::new(config.strategist.clone()) {
                    Ok(strategist) => Arc::new(strategist)
                        as Arc<dyn crate::strategist::SearchStrategist + Send + Sync>,
                    Err(error) => {
                        tracing::warn!(
                            target: "amatl::strategist",
                            error = %error,
                            "strategist backend is unavailable; falling back to no-op"
                        );
                        Arc::new(crate::strategist::NoOpStrategist)
                    }
                }
            } else {
                Arc::new(crate::strategist::NoOpStrategist)
            },
            _maintenance_cancel: maintenance_cancel,
        }
    }

    /// Registry backing this service.
    pub fn registry(&self) -> &ProviderRegistry {
        &self.registry
    }

    /// Build a replacement service from a new configuration, reusing this
    /// service's provider registry and mock flag.
    ///
    /// Nothing is mutated in place: the caller swaps the handle it hands to new
    /// requests, so in-flight work finishes against the configuration it
    /// started with. An invalid configuration is rejected before anything is
    /// rebuilt, so a bad reload leaves the running service untouched.
    pub async fn reloaded(self, config: Config) -> Result<Self, ServiceError> {
        config.validate().map_err(|_| ServiceError::Configuration)?;
        let ephemeral = self.ephemeral_credentials.clone();
        let mut replacement =
            Self::with_registry(config, self.mock, (*self.registry).clone()).await;
        // Keep operator-injected in-memory credentials across a reload so a
        // provider key survives config rotation without a restart.
        replacement.ephemeral_credentials = ephemeral;
        Ok(replacement)
    }

    /// [`Self::reloaded`], runnable from a generic async position.
    ///
    /// Opening SQLite runs migrations through `sqlx`, whose executor bound is
    /// not provably `Send` when the resulting future is awaited from a generic
    /// caller such as an HTTP handler. Driving that future on a blocking thread
    /// of the *current* runtime sidesteps the bound without detaching the
    /// connection pool from the runtime it belongs to. Reload is a rare,
    /// operator-triggered action, so the extra thread hop costs nothing that
    /// matters.
    pub async fn reloaded_detached(self, config: Config) -> Result<Self, ServiceError> {
        let handle = tokio::runtime::Handle::current();
        tokio::task::spawn_blocking(move || handle.block_on(self.reloaded(config)))
            .await
            .map_err(|_| ServiceError::Configuration)?
    }

    /// Build a replacement service with a different provider registry, for an
    /// embedder that adds or removes a source implementation at runtime.
    pub async fn reloaded_with_registry(
        self,
        config: Config,
        registry: ProviderRegistry,
    ) -> Result<Self, ServiceError> {
        config.validate().map_err(|_| ServiceError::Configuration)?;
        let ephemeral = self.ephemeral_credentials.clone();
        let mut replacement = Self::with_registry(config, self.mock, registry).await;
        replacement.ephemeral_credentials = ephemeral;
        Ok(replacement)
    }

    /// Identifier of the active inference backend, when one is available.
    pub fn inference_backend(&self) -> Option<&str> {
        self.inference.as_ref().map(InferenceRuntime::backend_id)
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    /// Limits in force for one surface, so a transport never hardcodes its own.
    pub fn execution_limits(&self, surface: ServiceSurface) -> ExecutionLimits {
        ExecutionLimits::for_surface(&self.config, surface)
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

    /// Run several independent searches in order and return one outcome per
    /// query.
    ///
    /// Each query executes exactly like a `/search` call (its own fresh
    /// orchestrator and budget, with circuit outcomes and history recorded per
    /// query against [`ServiceSurface::api`] semantics), but a failure in one
    /// query never aborts the rest: the batch is only as unavailable as its
    /// usable rows, and the fault is isolated to the failing item. The caller
    /// is responsible for bounding the batch size up front; per-query cost is
    /// already bounded by [`ExecutionLimits`], so `n` queries cost at most
    /// `n × budget`.
    pub async fn search_batch(
        &self,
        raw_queries: Vec<String>,
        surface: ServiceSurface,
    ) -> Vec<SearchBatchItem> {
        let mut items = Vec::with_capacity(raw_queries.len());
        for raw in raw_queries {
            let item = match self
                .search_paginated(raw.clone(), surface.clone(), None, None)
                .await
            {
                Ok(execution) => SearchBatchItem {
                    query: raw,
                    response: Some(execution.response),
                    error: None,
                },
                Err(error) => SearchBatchItem {
                    query: raw,
                    response: None,
                    error: Some(error.code()),
                },
            };
            items.push(item);
        }
        items
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
        let selection = self.select_providers()?;
        let refusals = selection.degradations;
        let providers = selection.providers;
        // Fresh orchestrator per search: a search mutates its internal budget,
        // so the refined (second) search must run on its own orchestrator with
        // its own budget rather than reusing the first, already-exhausted one.
        let build_orchestrator = || {
            SearchOrchestrator::new(
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
            .with_optics(self.optics.clone())
            .with_search_policy(self.config.search_policy.clone())
            .with_role_assignment(role_assignment_from_config(&self.config))
            .with_telemetry(self.telemetry.clone())
            .with_request_id(surface.request_id.clone())
        };
        let mut orchestrator = build_orchestrator();
        let mut response = orchestrator.search(query.clone(), providers.clone()).await;

        if self.config.strategist.enabled {
            if let Ok(Ok(Some(new_query))) = tokio::time::timeout(
                std::time::Duration::from_millis(self.config.strategist.timeout_ms),
                self.strategist
                    .generate_strategy(&query.raw_query, &response),
            )
            .await
            {
                if let Ok(refined_query) = crate::parse_query(new_query) {
                    // The refined query is a different, narrower query. If it
                    // normalizes to the same query the user already sent, the
                    // follow-up would add nothing: skip the redundant search.
                    if refined_query.normalized_query != query.normalized_query {
                        let mut refined_orchestrator = build_orchestrator();
                        let refined_response =
                            refined_orchestrator.search(refined_query, providers).await;
                        // Keep the circuit breaker accurate for the providers the
                        // refined search actually used, beyond the results merge.
                        self.record_circuit_outcomes(&refined_response).await;
                        // Complement, never re-order: keep the original set as
                        // ranked and append only results the original search did
                        // not already return, deduplicating by canonical URL
                        // (both against the original set and within the refined
                        // set itself). The refined round's provenance is merged
                        // too, so a refined round that recovered from a primary
                        // failure is not reported as `Failure`.
                        Self::merge_refined_response(&mut response, refined_response);
                    }
                }
            }
        }

        self.record_circuit_outcomes(&response).await;
        if !refusals.is_empty() {
            response.degradations.extend(refusals);
            if response.status == crate::SearchStatus::Success {
                response.status = crate::SearchStatus::PartialSuccess;
            }
        }
        if let Some(degradation) = &self.storage_degradation {
            response.degradations.push(degradation.clone());
            if response.status == crate::SearchStatus::Success {
                response.status = crate::SearchStatus::PartialSuccess;
            }
        }
        // Apply server-side pagination when requested. A lone `page` defaults
        // `page_size` to the surface maximum, and a lone `page_size` defaults
        // `page` to the first page: otherwise a client that sent one bound
        // would silently receive the full result set with no pagination
        // metadata.
        let (page, page_size) = match (page, page_size) {
            (None, None) => (None, None),
            (page, page_size) => (
                Some(page.unwrap_or(0)),
                Some(page_size.unwrap_or(limits.max_page_size)),
            ),
        };
        if let (Some(p), Some(ps)) = (page, page_size) {
            let ps = ps.clamp(1, limits.max_page_size);
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

    /// Append results from a refined (second) search to the primary result set,
    /// keeping the primary set as-ranked and adding only hits the primary
    /// search did not already return — deduplicating by canonical URL both
    /// against the primary set and within the refined set itself.
    ///
    /// `refined` is consumed. The primary set is mutated in place.
    fn append_complementary(primary: &mut Vec<SearchResult>, refined: Vec<SearchResult>) {
        let mut seen: BTreeSet<CanonicalUrl> = primary
            .iter()
            .map(|result| result.canonical_url.clone())
            .collect();
        primary.extend(
            refined
                .into_iter()
                .filter(|result| seen.insert(result.canonical_url.clone())),
        );
    }

    /// Fold a refined (strategist) response into `primary`: append its
    /// complementary results, merge its provenance, and recompute `status`
    /// from the combined set. Without this a refined round that recovered from
    /// a primary failure would leave `status: failure` alongside real results,
    /// and consumers keying off `status` would discard them.
    fn merge_refined_response(primary: &mut SearchResponse, refined: SearchResponse) {
        let SearchResponse {
            providers_used,
            providers_failed,
            providers_partial,
            errors,
            degradations,
            results,
            ..
        } = refined;
        Self::append_complementary(&mut primary.results, results);
        for provider in providers_used {
            if !primary.providers_used.contains(&provider) {
                primary.providers_used.push(provider);
            }
        }
        for provider in providers_failed {
            if !primary.providers_failed.contains(&provider) {
                primary.providers_failed.push(provider);
            }
        }
        for provider in providers_partial {
            if !primary.providers_partial.contains(&provider) {
                primary.providers_partial.push(provider);
            }
        }
        primary.errors.extend(errors);
        primary.degradations.extend(degradations);
        // Recompute status from the combined result set, mirroring the rule the
        // orchestrator applies before the strategist runs.
        if primary.results.is_empty() {
            primary.status = crate::SearchStatus::Failure;
        } else {
            // A `no_usable_results` error from a primary round that the refined
            // round recovered from is now stale.
            primary
                .errors
                .retain(|error| error.code != "no_usable_results");
            primary.status = if primary.degradations.is_empty()
                && primary.providers_failed.is_empty()
                && primary.providers_partial.is_empty()
            {
                crate::SearchStatus::Success
            } else {
                crate::SearchStatus::PartialSuccess
            };
        }
    }

    /// Run a search and synthesize a grounded, cited answer from its results.
    ///
    /// Distinct from [`Self::search`] on purpose: this is the only surface
    /// that calls out to an LLM, and only when `answer.enabled` in
    /// configuration explicitly turns it on. `search`/`deep` never change
    /// behavior because this exists. See [`crate::answer`] for the grounding
    /// contract enforced on the model's response.
    pub async fn answer(
        &self,
        raw_query: String,
        surface: ServiceSurface,
    ) -> Result<AnswerResult, ServiceError> {
        let backend = self
            .answer_backend
            .as_ref()
            .ok_or(ServiceError::AnswerUnavailable)?;
        let search = self
            .search_inner(raw_query.clone(), surface.clone(), None, None)
            .await?;
        self.record_history(&search, 0, surface).await;
        let sources = crate::answer::build_sources(
            &search.response.results,
            self.config.answer.max_sources,
            self.config.answer.max_source_chars,
        );
        let answer = crate::answer::synthesize(backend.as_ref(), &raw_query, sources)
            .await
            .map_err(|_: AnswerError| ServiceError::AnswerUnavailable)?;
        Ok(AnswerResult {
            schema_version: SCHEMA_VERSION.into(),
            answer,
            search: search.response,
        })
    }

    pub async fn deep(
        &self,
        raw_query: String,
        surface: ServiceSurface,
    ) -> Result<DeepResponse, ServiceError> {
        self.deep_with_fetch_cap(raw_query, None, surface).await
    }

    /// Run the legacy Search-backed Deep flow with an optional request cap.
    /// The cap can only narrow the configured surface budget.
    pub async fn deep_with_fetch_cap(
        &self,
        raw_query: String,
        max_fetches: Option<u32>,
        surface: ServiceSurface,
    ) -> Result<DeepResponse, ServiceError> {
        let limits = ExecutionLimits::for_surface(&self.config, surface.clone());
        validate_deep_fetch_cap(max_fetches, limits.deep_max_fetches)?;
        let search = self
            .search_inner(raw_query, surface.clone(), None, None)
            .await?;
        self.deep_from_search(search, max_fetches, surface).await
    }

    /// Run Deep from explicit Search-result identities without re-running
    /// Search. Targets are canonicalized and deduplicated before they reach
    /// the shared Deep budget owner.
    pub async fn deep_selected(
        &self,
        raw_query: String,
        targets: Vec<DeepTarget>,
        max_fetches: Option<u32>,
        surface: ServiceSurface,
    ) -> Result<DeepResponse, ServiceError> {
        let limits = ExecutionLimits::for_surface(&self.config, surface.clone());
        validate_deep_fetch_cap(max_fetches, limits.deep_max_fetches)?;
        if targets.is_empty() || targets.len() > self.config.deep.top_k as usize {
            return Err(ServiceError::InvalidInput);
        }

        let query = parse_query(raw_query).map_err(|_| ServiceError::InvalidQuery)?;
        let mut seen = BTreeSet::new();
        let mut candidates = Vec::new();
        for target in targets {
            if target.provider.is_empty()
                || target.provider.len() > 128
                || target.title.as_ref().is_some_and(|title| title.len() > 512)
                || !self.target_provider_is_valid(&target.provider)
            {
                return Err(ServiceError::InvalidInput);
            }
            let url = crate::security::validate_search_url(&target.url)
                .map_err(|_| ServiceError::InvalidInput)?;
            let canonicalized = crate::canonical::canonicalize(crate::NormalizedResult {
                schema_version: SCHEMA_VERSION.into(),
                title: target.title.clone(),
                raw_url: target.url,
                url: crate::OriginalUrl(url),
                provider: target.provider.clone(),
                provider_rank: None,
                snippet: None,
                result_type: crate::ResultType::Organic,
                published_at: None,
                author: None,
                language: None,
                file_type: None,
                thumbnail: None,
                metadata: BTreeMap::new(),
                provenance: BTreeMap::new(),
                degradations: vec![],
            });
            let original_url = canonicalized.original_url;
            let canonical = canonicalized.canonical_url;
            if !seen.insert(canonical.0.as_str().to_owned()) {
                continue;
            }
            let rank =
                Rank::new(candidates.len() as u32 + 1).map_err(|_| ServiceError::InvalidInput)?;
            let storage_rights =
                !self.mock && self.config.providers.storage_rights(&target.provider);
            candidates.push(DeepCandidate {
                result: SearchResult {
                    schema_version: SCHEMA_VERSION.into(),
                    rank,
                    title: target.title,
                    original_url,
                    domain: canonical
                        .0
                        .host_str()
                        .ok_or(ServiceError::InvalidInput)?
                        .to_owned(),
                    canonical_url: CanonicalUrl(canonical.0),
                    snippet: None,
                    providers: vec![target.provider],
                    published_at: None,
                    status: crate::ResultStatus::Visible,
                },
                storage_rights,
            });
        }
        if candidates.is_empty() {
            return Err(ServiceError::InvalidInput);
        }

        let mut planning_budget = Budget::new(limits.max_provider_calls, limits.search_timeout_ms);
        let plan = crate::planning::build_search_plan(
            query.clone(),
            crate::classify(&query),
            crate::router::RoutingRecommendation {
                selected_providers: vec![],
                provider_budget_requests: BTreeMap::new(),
                debug_reasons: vec!["selected_search_targets".into()],
            },
            &mut planning_budget,
        );
        let response = SearchResponse {
            schema_version: SCHEMA_VERSION.into(),
            query: query.raw_query.clone(),
            status: crate::SearchStatus::Success,
            results: candidates
                .iter()
                .map(|candidate| candidate.result.clone())
                .collect(),
            providers_used: vec![],
            providers_failed: vec![],
            providers_partial: vec![],
            errors: vec![],
            degradations: vec![],
            elapsed_ms: 0,
            total_results: None,
            page: None,
            page_size: None,
            complementarity: None,
        };
        self.deep_from_search(
            SearchExecution {
                query,
                plan,
                response,
            },
            max_fetches,
            surface,
        )
        .await
    }

    fn target_provider_is_valid(&self, provider: &str) -> bool {
        (self.mock
            && mock_providers()
                .iter()
                .any(|source| source.name() == provider))
            || (self.config.providers.is_declared(provider) && self.registry.contains(provider))
    }

    async fn deep_from_search(
        &self,
        search: SearchExecution,
        request_max_fetches: Option<u32>,
        surface: ServiceSurface,
    ) -> Result<DeepResponse, ServiceError> {
        let history_execution = search.clone();
        let limits = ExecutionLimits::for_surface(&self.config, surface.clone());
        let effective_max_fetches = request_max_fetches.unwrap_or(limits.deep_max_fetches);
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
                        model_version: self.inference.as_ref().map(InferenceRuntime::version_key),
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
            effective_max_fetches,
            limits.deep_max_bytes,
            self.config.deep.max_redirects,
            self.config.deep.renderer.max_browser_calls,
            self.config.deep.max_crawl_urls,
            remaining_deep_ms,
        )
        .with_gap_limits(limits.deep_max_subqueries, limits.deep_max_subquery_cost);
        let extractor = Arc::new(FallbackExtractor::new(
            TrafilaturaExtractor::new(
                self.config.deep.extractor.executable.clone(),
                self.config.deep.extractor.version.clone(),
                self.config.deep.extractor.timeout_ms.min(remaining_deep_ms),
                self.config.deep.extractor.max_output_bytes,
            ),
            NativeHtmlExtractor::new(self.config.deep.extractor.max_output_bytes),
        ));
        let mut orchestrator = DeepOrchestrator::new(
            deep_budget,
            self.fetcher.clone(),
            extractor,
            self.renderer_pool.clone(),
            document_cache,
            remaining_deep_ms,
            (limits.deep_max_bytes / u64::from(effective_max_fetches)).max(1),
            self.config.deep.max_redirects,
            self.config.deep.top_k as usize,
            self.config.deep.max_depth,
        )
        .with_request_id(surface.request_id.clone());
        // Crawl politeness applies to discovered links only, and only when the
        // crawl can actually reach depth.
        if self.config.deep.respect_robots && self.config.deep.max_depth > 0 {
            orchestrator = orchestrator.with_robots(crate::robots::RobotsCache::new(
                self.fetcher.clone(),
                self.config.deep.robots_timeout_ms,
                self.config.deep.robots_max_bytes,
            ));
        }
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

    /// Write an on-demand backup right now, identical to what the periodic
    /// maintenance task writes — see
    /// [`crate::storage::SqliteStorage::trigger_backup`]. Returns the path
    /// written.
    pub async fn trigger_backup(&self) -> Result<std::path::PathBuf, ServiceError> {
        self.storage_or_error()?
            .trigger_backup(
                self.config.persistence.backup_directory.as_deref(),
                self.config.persistence.auto_backup_max_count,
            )
            .await
            .map_err(|_| ServiceError::StorageUnavailable)
    }

    /// Backup files on disk for the configured database, newest first.
    pub fn list_backups(&self) -> Result<Vec<std::path::PathBuf>, ServiceError> {
        if !self.config.persistence.enabled {
            return Err(ServiceError::StorageUnavailable);
        }
        SqliteStorage::list_backups(
            std::path::Path::new(&self.config.persistence.path),
            self.config.persistence.backup_directory.as_deref(),
        )
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

    /// In-memory telemetry health: sample count and how many `storage`
    /// writes have failed since start. `record`'s memory and storage writes
    /// are deliberately independent (see its doc comment), so this is the
    /// only visibility into how often they've drifted apart.
    pub fn telemetry_status(&self) -> crate::telemetry::TelemetryStatus {
        self.telemetry.status()
    }

    /// Operator status: source availability, persistence and cache state.
    pub async fn status(&self) -> Result<ServiceStatus, ServiceError> {
        let snapshots = self.source_snapshots();
        let circuits = self.circuit_snapshots();
        let (sources, summaries_unavailable) = match self.provider_summaries() {
            Ok(summaries) => {
                let sources = summaries
                    .into_iter()
                    .map(|summary| {
                        let snapshot = snapshots
                            .iter()
                            .find(|snapshot| snapshot.provider == summary.name);
                        let circuit = circuits
                            .iter()
                            .find(|snapshot| snapshot.provider == summary.name)
                            .map(|snapshot| snapshot.state)
                            .unwrap_or(crate::circuit::CircuitState::Closed);
                        SourceStatus {
                            name: summary.name,
                            status: summary.status,
                            code: summary.code,
                            circuit,
                            success_rate: snapshot
                                .filter(|snapshot| snapshot.sample > 0)
                                .map(|snapshot| snapshot.success_rate),
                            average_latency_ms: snapshot
                                .filter(|snapshot| snapshot.sample > 0)
                                .map(|snapshot| snapshot.average_latency_ms),
                            observations: snapshot.map(|snapshot| snapshot.sample).unwrap_or(0),
                            credential_source: summary.credential_source,
                        }
                    })
                    .collect::<Vec<_>>();
                (sources, false)
            }
            Err(error) => {
                // Diagnostics must survive the failure they are meant to
                // diagnose: when the HTTP transport failed to initialize,
                // report every declared source as unavailable instead of
                // failing the whole status call.
                tracing::warn!(
                    target: "amatl::service",
                    error = %error,
                    "provider summaries unavailable; reporting declared sources as unavailable"
                );
                let sources = self
                    .config
                    .providers
                    .declared()
                    .map(|(name, runtime)| {
                        let circuit = circuits
                            .iter()
                            .find(|snapshot| snapshot.provider == *name)
                            .map(|snapshot| snapshot.state)
                            .unwrap_or(crate::circuit::CircuitState::Closed);
                        let credential_source = if self.ephemeral_credential_present(name) {
                            CredentialSource::Ephemeral
                        } else if credential(runtime).is_some() {
                            CredentialSource::Environment
                        } else {
                            CredentialSource::None
                        };
                        SourceStatus {
                            name: name.to_owned(),
                            status: ProviderSurfaceStatus::Unavailable,
                            code: Some("transport_unavailable".into()),
                            circuit,
                            success_rate: None,
                            average_latency_ms: None,
                            observations: 0,
                            credential_source,
                        }
                    })
                    .collect::<Vec<_>>();
                (sources, true)
            }
        };
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
        let degraded = summaries_unavailable
            || storage.enabled && !storage.available
            || sources.iter().any(|source| {
                source.status != ProviderSurfaceStatus::Available
                    || source.circuit != crate::circuit::CircuitState::Closed
            });
        Ok(ServiceStatus {
            schema_version: SCHEMA_VERSION.into(),
            status: if degraded { "degraded" } else { "ok" }.into(),
            sources,
            storage,
            cache,
            inference_backend: self.inference_backend().map(str::to_owned),
            answer: AnswerStatus {
                enabled: self.config.answer.enabled,
                configured: self.config.answer.endpoint.is_some()
                    && self.config.answer.model.is_some(),
                available: self.answer_backend.is_some(),
                model: self.config.answer.model.clone(),
                endpoint: self.config.answer.endpoint.clone(),
            },
            strategist: StrategistStatus {
                enabled: self.config.strategist.enabled,
                configured: self.config.strategist.endpoint.is_some()
                    && self.config.strategist.model.is_some(),
                available: self.config.strategist.enabled
                    && self.config.strategist.endpoint.is_some()
                    && self.config.strategist.model.is_some()
                    && self
                        .config
                        .strategist
                        .credential_env
                        .as_ref()
                        .is_some_and(|name| std::env::var_os(name).is_some_and(|v| !v.is_empty())),
                model: self.config.strategist.model.clone(),
                endpoint: self.config.strategist.endpoint.clone(),
            },
            data_policy: DataPolicyStatus {
                profile: self.config.data_policy.profile,
                egress: self.config.data_policy.egress,
                inference: self.config.data_policy.inference,
            },
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
        let entries = self
            .config
            .providers
            .declared()
            .filter_map(|(name, runtime)| {
                let factory = self.registry.get(name)?;
                let credential_source = if self.ephemeral_credential_present(name) {
                    CredentialSource::Ephemeral
                } else if credential(runtime).is_some() {
                    CredentialSource::Environment
                } else {
                    CredentialSource::None
                };
                Some((
                    factory.build(&ProviderBuildContext {
                        name,
                        runtime,
                        enabled: self.config.providers.enabled.contains(name),
                        approved: runtime.approved(),
                        credential: self.resolved_credential(name, runtime),
                        transport: transport.clone(),
                    }),
                    credential_source,
                ))
            })
            .collect::<Vec<(Arc<dyn Provider>, CredentialSource)>>();
        Ok(entries
            .into_iter()
            .map(|(provider, credential_source)| {
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
                    enabled: self
                        .config
                        .providers
                        .enabled
                        .iter()
                        .any(|name| name == provider.name()),
                    credential_source,
                }
            })
            .collect())
    }

    /// Resolve a provider's credential: an operator-supplied ephemeral
    /// (RAM-only) value takes precedence; otherwise fall back to the
    /// `credential_env` environment variable. Never logs the secret. The value
    /// is `Zeroizing`, so the per-build copy handed to a provider is wiped from
    /// the heap when that provider is dropped.
    fn resolved_credential(
        &self,
        name: &str,
        runtime: &ProviderRuntimeConfig,
    ) -> Option<Zeroizing<String>> {
        if let Ok(guard) = self.ephemeral_credentials.read() {
            if let Some(value) = guard.get(name) {
                if !value.is_empty() {
                    return Some(value.clone());
                }
            }
        }
        credential(runtime)
    }

    /// Whether an operator has injected an in-memory credential for `name`.
    /// Read-only; never exposes the secret, only its presence — and it checks
    /// presence without cloning the key.
    pub fn ephemeral_credential_present(&self, name: &str) -> bool {
        self.ephemeral_credentials
            .read()
            .map(|guard| guard.get(name).is_some_and(|value| !value.is_empty()))
            .unwrap_or(false)
    }

    /// Set an operator-supplied provider credential in memory only.
    ///
    /// Nothing is written to disk, to `amatl.toml`, the environment, or logs:
    /// the value lives in RAM and dies with the process. A generous upper bound
    /// rejects a runaway value; an empty value is rejected too.
    pub fn set_provider_credential(&self, name: &str, value: &str) -> Result<(), ServiceError> {
        if self.registry.get(name).is_none() {
            return Err(ServiceError::ProviderNotRegistered(name.into()));
        }
        if value.is_empty() || value.len() > 512 {
            return Err(ServiceError::InvalidInput);
        }
        self.ephemeral_credentials
            .write()
            .map_err(|_| ServiceError::ProviderNotRegistered(name.into()))?
            .insert(name.to_string(), Zeroizing::new(value.to_owned()));
        Ok(())
    }

    /// Remove an operator-supplied provider credential from memory. Idempotent.
    ///
    /// `remove` drops the `Zeroizing` value, which wipes its bytes before the
    /// allocation is released, so the liquidated key does not linger in the
    /// heap for a later allocation (or a core dump) to expose.
    pub fn clear_provider_credential(&self, name: &str) {
        if let Ok(mut guard) = self.ephemeral_credentials.write() {
            drop(guard.remove(name));
        }
    }

    /// Live, bounded probe of a candidate provider credential.
    ///
    /// Builds an isolated service with only `name` enabled and the candidate
    /// key injected in-memory, then runs one search. Returns `true` only when
    /// the source actually produced at least one usable result, so an operator
    /// never accepts (or "burns") a bogus, expired, or mis-scoped key. Fails
    /// closed (`false`) when egress is denied. The key is never logged.
    pub async fn probe_provider_credential(
        &self,
        name: &str,
        value: &str,
        query: &str,
    ) -> Result<bool, ServiceError> {
        if !self.config.data_policy.allows_network_egress() {
            return Ok(false);
        }
        let mut isolated_config = (*self.config).clone();
        isolated_config.providers.enabled = vec![name.to_string()];
        let registry = (*self.registry).clone();
        let name = name.to_string();
        let value = Zeroizing::new(value.to_owned());
        let query = query.to_string();
        // Opening SQLite runs migrations through sqlx, whose executor bound is
        // not provably `Send`; build the isolated service on a blocking thread
        // (same reason `reloaded_detached` does) so this method's future stays
        // `Send` and callable from an HTTP handler.
        let handle = tokio::runtime::Handle::current();
        let service = tokio::task::spawn_blocking(move || {
            handle.block_on(AmatlService::with_registry(
                isolated_config,
                false,
                registry,
            ))
        })
        .await
        .map_err(|_| ServiceError::Configuration)?;
        service
            .set_provider_credential(&name, &value)
            .map_err(|_| ServiceError::ProviderNotRegistered(name.clone()))?;
        let execution = service.search(query.clone(), ServiceSurface::cli()).await?;
        let used = execution
            .response
            .providers_used
            .iter()
            .any(|used| used == &name);
        Ok(
            !matches!(execution.response.status, crate::SearchStatus::Failure)
                && used
                && !execution.response.results.is_empty(),
        )
    }

    /// Providers admitted into one search round, with the reasons for any
    /// source that was refused.
    ///
    /// Two runtime gates apply on top of configuration. Governance: an enabled
    /// provider whose approval record is incomplete or expired is never built,
    /// so a stale record cannot start sending traffic just because the name is
    /// listed in `providers.enabled`. Circuit: a source whose breaker is open
    /// is skipped until its cooldown expires. Both produce a degradation
    /// instead of a silent omission.
    fn select_providers(&self) -> Result<ProviderSelection, ServiceError> {
        if self.mock {
            return Ok(ProviderSelection {
                providers: mock_providers(),
                degradations: vec![],
            });
        }
        if !self.config.data_policy.allows_network_egress() {
            return Ok(ProviderSelection {
                providers: vec![],
                degradations: vec![crate::Degradation {
                    code: ErrorCode::EgressDenied.as_str().into(),
                    component: "data_policy".into(),
                    message:
                        "outbound network access is denied by data policy; no source was called"
                            .into(),
                }],
            });
        }
        let transport = self.transport.clone().ok_or(ServiceError::Configuration)?;
        let now = now_unix();
        let mut selection = ProviderSelection::default();
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
            if !runtime.approved() {
                selection.degradations.push(crate::Degradation {
                    code: ErrorCode::ProviderNotApproved.as_str().into(),
                    component: name.clone(),
                    message: format!(
                        "{name} is enabled but its governance record is incomplete or expired; \
                         the source was not called"
                    ),
                });
                tracing::warn!(
                    target: "amatl::providers",
                    provider = %name,
                    "enabled provider refused by the governance gate"
                );
                continue;
            }
            // Claims the single half-open probe if the cooldown has expired, so
            // concurrent searches admit one probe, not a stampede.
            if !self.circuit.allows_call(name, now) {
                selection.degradations.push(crate::Degradation {
                    code: "provider_circuit_open".into(),
                    component: name.clone(),
                    message: format!(
                        "{name} is in circuit cooldown after consecutive failures; \
                         the source was skipped"
                    ),
                });
                continue;
            }
            let provider = factory.build(&ProviderBuildContext {
                name,
                runtime,
                enabled: true,
                approved: true,
                credential: self.resolved_credential(name, runtime),
                transport: transport.clone(),
            });
            selection.providers.push(self.with_cache(provider, runtime));
        }
        Ok(selection)
    }

    /// Providers admitted into one search round.
    fn providers(&self) -> Result<Vec<Arc<dyn Provider>>, ServiceError> {
        Ok(self.select_providers()?.providers)
    }

    /// Durable security audit trail for this service.
    pub fn audit(&self) -> &SecurityAudit {
        &self.audit
    }

    /// Circuit state of every provider the breaker has observed.
    pub fn circuit_snapshots(&self) -> Vec<CircuitSnapshot> {
        self.circuit.snapshots(now_unix())
    }

    /// Close every circuit; for an operator that fixed the underlying cause.
    pub async fn reset_circuits(&self) {
        self.circuit.reset().await;
    }

    /// Feed one search outcome into the breaker, one entry per attempted
    /// source. A partial provider still counts as a success: it answered.
    async fn record_circuit_outcomes(&self, response: &SearchResponse) {
        let now = now_unix();
        for provider in &response.providers_used {
            self.circuit.record(provider, true, now).await;
        }
        for provider in &response.providers_failed {
            // A provider that was never called because the local concurrency
            // deadline expired is not a provider failure: recording it would
            // open the breaker on a healthy source after a few busy searches.
            let locally_skipped = response.errors.iter().any(|error| {
                error.message == crate::execution::CONCURRENCY_DEADLINE_MESSAGE
                    && error.providers.iter().any(|name| name == provider)
            });
            if locally_skipped {
                continue;
            }
            self.circuit.record(provider, false, now).await;
        }
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

fn credential(config: &ProviderRuntimeConfig) -> Option<Zeroizing<String>> {
    config
        .credential_env
        .as_deref()
        .and_then(|name| std::env::var(name).ok())
        .filter(|value| !value.is_empty())
        .map(Zeroizing::new)
}

/// Build the router's [`RoleAssignment`] from `[expansion]` config.
///
/// Returns [`RoleAssignment::legacy`] unless `expansion.mode =
/// "primary_expansion"`, so a default config routes exactly as it did before
/// STEP 1.
fn role_assignment_from_config(config: &Config) -> RoleAssignment {
    if config.expansion.role_model_active() {
        RoleAssignment::primary_expansion(
            config.expansion.primary_provider.clone(),
            config.expansion.expansion_providers.clone(),
        )
    } else {
        RoleAssignment::legacy()
    }
}

/// Loads and parses the operator Optics file named by `[ranking.optics]`.
///
/// Returns `None` -- Optics simply inert -- when it is disabled, has no path,
/// the file cannot be read, or the file does not parse. The last two log a
/// warning: a misconfigured `.optic` must never fail a search, only be
/// ignored.
fn load_optics(config: &crate::config::OpticsConfig) -> Option<Arc<crate::optics::OpticsDocument>> {
    if !config.enabled {
        return None;
    }
    let Some(path) = config.path.as_deref() else {
        tracing::warn!(
            target: "amatl::optics",
            "[ranking.optics] is enabled but no path is set; optics disabled"
        );
        return None;
    };
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) => {
            tracing::warn!(
                target: "amatl::optics",
                path = %path,
                error = %error,
                "optics file could not be read; optics disabled"
            );
            return None;
        }
    };
    match crate::optics::parse_optics(&text) {
        Ok(document) => {
            tracing::info!(
                target: "amatl::optics",
                path = %path,
                rules = document.rules.len(),
                discard_non_matching = document.discard_non_matching,
                "loaded operator optics"
            );
            Some(Arc::new(document))
        }
        Err(error) => {
            tracing::warn!(
                target: "amatl::optics",
                path = %path,
                error = %error,
                "optics file did not parse; optics disabled"
            );
            None
        }
    }
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

    #[test]
    fn request_fetch_cap_only_narrows_the_configured_budget() {
        assert!(validate_deep_fetch_cap(None, 10).is_ok());
        assert!(validate_deep_fetch_cap(Some(3), 10).is_ok());
        assert!(validate_deep_fetch_cap(Some(10), 10).is_ok());
        assert_eq!(
            validate_deep_fetch_cap(Some(11), 10),
            Err(ServiceError::InvalidInput)
        );
        assert_eq!(
            validate_deep_fetch_cap(Some(0), 10),
            Err(ServiceError::InvalidInput)
        );
    }

    #[test]
    fn append_complementary_dedupes_and_preserves_primary_order() {
        use crate::{CanonicalUrl, OriginalUrl, Rank, ResultStatus, SearchResult};
        use url::Url;
        let result = |index: u32, host: &str, path: &str| SearchResult {
            schema_version: SCHEMA_VERSION.into(),
            rank: Rank::new(index).unwrap(),
            title: None,
            original_url: OriginalUrl(Url::parse(&format!("http://{host}/{path}")).unwrap()),
            canonical_url: CanonicalUrl(Url::parse(&format!("http://{host}/{path}")).unwrap()),
            domain: host.to_owned(),
            snippet: None,
            providers: vec!["mock".into()],
            published_at: None,
            status: ResultStatus::Visible,
        };
        // Primary set, already ranked.
        let mut primary = vec![result(1, "a.example", "one"), result(2, "b.example", "two")];
        // Refined set: one duplicate of the primary, one duplicate within the
        // refined set itself, and one genuinely new result.
        let refined = vec![
            result(1, "a.example", "one"),
            result(9, "c.example", "new-a"),
            result(9, "c.example", "new-a"),
            result(9, "d.example", "new-b"),
        ];
        AmatlService::append_complementary(&mut primary, refined);
        let urls = primary
            .iter()
            .map(|item| item.canonical_url.0.as_str().to_owned())
            .collect::<Vec<_>>();
        assert_eq!(
            urls,
            [
                "http://a.example/one",
                "http://b.example/two",
                "http://c.example/new-a",
                "http://d.example/new-b",
            ],
            "primary order is preserved, duplicates are dropped, new hits are appended"
        );
    }

    /// Same registered name as [`ArchiveFactory`], but the source always fails,
    /// so the breaker has something real to trip on.
    struct FailingFactory;

    impl ProviderFactory for FailingFactory {
        fn name(&self) -> &str {
            "custom_archive"
        }

        fn requires_credential(&self) -> bool {
            false
        }

        fn build(&self, context: &ProviderBuildContext<'_>) -> Arc<dyn Provider> {
            Arc::new(MockProvider::new(
                context.name,
                crate::MockBehavior::Failure(crate::ProviderErrorKind::Unavailable),
            ))
        }
    }

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

    /// A provider whose availability depends on a resolved credential. When the
    /// credential is absent it reports `credential_missing` — the same fallback
    /// contract as the real Marginalia adapter — so the orchestrator must
    /// degrade it instead of failing the round.
    struct CredentialGatedFactory;

    impl ProviderFactory for CredentialGatedFactory {
        fn name(&self) -> &str {
            "credential_gated"
        }

        fn requires_credential(&self) -> bool {
            true
        }

        fn build(&self, context: &ProviderBuildContext<'_>) -> Arc<dyn Provider> {
            Arc::new(CredentialGatedProvider {
                name: context.name.into(),
                credential: context.credential.clone(),
            })
        }
    }

    struct CredentialGatedProvider {
        name: String,
        credential: Option<Zeroizing<String>>,
    }

    #[async_trait]
    impl Provider for CredentialGatedProvider {
        fn name(&self) -> &str {
            &self.name
        }

        fn capabilities(&self) -> ProviderCapabilities {
            ProviderCapabilities {
                schema_version: SCHEMA_VERSION.into(),
                pagination: false,
                language: false,
                region: false,
                time_range: false,
                site_filter: false,
                file_filter: false,
                news: false,
                code: false,
                docs: true,
                academic: false,
                authentication: true,
                estimated_cost: Some(0),
            }
        }

        fn availability(&self) -> ProviderAvailability {
            if self.credential.is_none() {
                ProviderAvailability::Unavailable {
                    code: "credential_missing".into(),
                    message: "credential is not set".into(),
                }
            } else {
                ProviderAvailability::Available
            }
        }

        async fn search(
            &self,
            _plan: &crate::SearchPlan,
            _context: &crate::ProviderContext,
        ) -> Result<crate::ProviderResult, crate::ProviderError> {
            Ok(crate::ProviderResult {
                schema_version: SCHEMA_VERSION.into(),
                provider: self.name.clone(),
                status: crate::ProviderExecutionStatus::Success,
                results: vec![],
                accepted_filters: vec![],
                ignored_filters: vec![],
                approximated_filters: vec![],
                errors: vec![],
            })
        }
    }

    /// Governance record that satisfies the runtime approval gate today.
    fn approved_record(adapter_version: &str) -> ProviderRuntimeConfig {
        ProviderRuntimeConfig {
            adapter_version: Some(adapter_version.into()),
            approval_status: crate::ApprovalStatus::Approved,
            reviewed_at: Some(today_iso()),
            reviewer: Some("owner".into()),
            terms_url: Some("https://archive.invalid/terms".into()),
            terms_version_or_date: Some("2026-08-01".into()),
            allowed_access_method: Some("official_api".into()),
            plan_or_contract: Some("contract-1".into()),
            rate_limit: Some("1 qps".into()),
            cost_model: Some("free".into()),
            data_handling_notes: Some("no cache".into()),
            operational_risk: Some("none".into()),
            ..ProviderRuntimeConfig::default()
        }
    }

    fn archive_config() -> Config {
        let mut config = Config::default();
        config
            .providers
            .declare("custom_archive", approved_record("archive-v1"));
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

    #[tokio::test]
    async fn ephemeral_credential_lives_in_memory_until_liquidated() {
        let service = AmatlService::new(Config::default(), false).await;
        assert!(!service.ephemeral_credential_present("marginalia"));

        service
            .set_provider_credential("marginalia", "marginalia-secret-key")
            .unwrap();
        assert!(service.ephemeral_credential_present("marginalia"));

        service.clear_provider_credential("marginalia");
        assert!(!service.ephemeral_credential_present("marginalia"));

        // Idempotent: liquidating an absent key is not an error.
        service.clear_provider_credential("marginalia");
        assert!(!service.ephemeral_credential_present("marginalia"));
    }

    #[tokio::test]
    async fn ephemeral_credential_is_never_serialized_into_the_config() {
        let service = AmatlService::new(Config::default(), false).await;
        service
            .set_provider_credential("marginalia", "marginalia-secret-key")
            .unwrap();
        // The key must never reach a serialized form of the configuration — the
        // same shape a config file or a `/config` view would carry.
        let serialized = serde_json::to_string(service.config()).unwrap();
        assert!(!serialized.contains("marginalia-secret-key"));
    }

    #[tokio::test]
    async fn ephemeral_credential_rejects_unregistered_empty_and_oversized_values() {
        let service = AmatlService::new(Config::default(), false).await;
        assert!(service
            .set_provider_credential("not_registered", "k")
            .is_err());
        assert!(service.set_provider_credential("marginalia", "").is_err());
        assert!(service
            .set_provider_credential("marginalia", &"x".repeat(513))
            .is_err());
        // None of the rejected values were committed.
        assert!(!service.ephemeral_credential_present("marginalia"));
    }

    #[tokio::test]
    async fn ephemeral_credential_survives_reload_without_a_restart() {
        let service = AmatlService::new(Config::default(), false).await;
        service
            .set_provider_credential("marginalia", "marginalia-secret-key")
            .unwrap();
        let reloaded = service.reloaded(Config::default()).await.unwrap();
        assert!(reloaded.ephemeral_credential_present("marginalia"));
    }

    #[tokio::test]
    async fn credential_source_reports_none_then_ephemeral() {
        let mut config = Config::default();
        // Point marginalia at a variable that is not set, so the baseline is
        // `none` regardless of the host environment (an operator shell may have
        // MARGINALIA_API_KEY exported).
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        config.providers.entry("marginalia").credential_env =
            Some(format!("AMATL_TEST_UNSET_{nonce}"));
        let service = AmatlService::new(config, false).await;
        let source = |service: &AmatlService| {
            service
                .provider_summaries()
                .unwrap()
                .into_iter()
                .find(|summary| summary.name == "marginalia")
                .unwrap()
                .credential_source
        };
        assert_eq!(source(&service), CredentialSource::None);

        service
            .set_provider_credential("marginalia", "marginalia-secret-key")
            .unwrap();
        assert_eq!(source(&service), CredentialSource::Ephemeral);
    }

    #[tokio::test]
    async fn credential_source_reports_environment_when_only_the_variable_is_set() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let var = format!("AMATL_TEST_MARGINALIA_KEY_{nonce}");
        std::env::set_var(&var, "environment-supplied-key");

        let mut config = Config::default();
        config.providers.entry("marginalia").credential_env = Some(var.clone());
        let service = AmatlService::new(config, false).await;
        let summary = service
            .provider_summaries()
            .unwrap()
            .into_iter()
            .find(|summary| summary.name == "marginalia")
            .unwrap();
        assert_eq!(summary.credential_source, CredentialSource::Environment);

        std::env::remove_var(&var);
    }

    #[tokio::test]
    async fn status_sources_expose_credential_provenance_but_not_the_value() {
        let service = AmatlService::new(Config::default(), false).await;
        service
            .set_provider_credential("marginalia", "marginalia-secret-key")
            .unwrap();
        let status = service.status().await.unwrap();
        let marginalia = status
            .sources
            .iter()
            .find(|source| source.name == "marginalia")
            .unwrap();
        assert_eq!(marginalia.credential_source, CredentialSource::Ephemeral);
        // The operator view names the provenance, never the key.
        let serialized = serde_json::to_string(&status).unwrap();
        assert!(!serialized.contains("marginalia-secret-key"));
    }

    #[test]
    fn credential_source_serializes_to_the_contracted_wire_values() {
        assert_eq!(
            serde_json::to_string(&CredentialSource::None).unwrap(),
            "\"none\""
        );
        assert_eq!(
            serde_json::to_string(&CredentialSource::Ephemeral).unwrap(),
            "\"ephemeral\""
        );
        assert_eq!(
            serde_json::to_string(&CredentialSource::Environment).unwrap(),
            "\"environment\""
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
    }

    #[tokio::test]
    async fn missing_credential_degrades_the_provider_instead_of_failing_the_search() {
        let mut config = Config::default();
        config.providers.declare(
            "credential_gated",
            ProviderRuntimeConfig {
                credential_env: Some("AMATL_TEST_CREDENTIAL_NEVER_SET".into()),
                ..approved_record("credential-gated-v1")
            },
        );
        config
            .providers
            .declare("custom_archive", approved_record("archive-v1"));
        config.providers.enabled = vec!["credential_gated".into(), "custom_archive".into()];
        // The credential env var is intentionally left unset: `credential()`
        // resolves it to `None`, which must degrade the provider rather than
        // fail the search. The name is unique to this test and never exported.
        let service = AmatlService::with_registry(
            config,
            false,
            ProviderRegistry::builtin()
                .with(Arc::new(CredentialGatedFactory))
                .with(Arc::new(ArchiveFactory)),
        )
        .await;
        let execution = service
            .search("rust".into(), ServiceSurface::cli())
            .await
            .unwrap();
        // The working provider answered, so the search is not a hard failure.
        assert_eq!(
            execution.response.providers_used,
            vec!["custom_archive".to_string()]
        );
        assert!(execution.response.providers_failed.is_empty());
        // The missing credential degraded the gated provider instead of failing it.
        assert!(execution
            .response
            .degradations
            .iter()
            .any(|degradation| degradation.code == "credential_missing"
                && degradation.component == "credential_gated"));
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
        // Start from an incomplete record so the governance step is exercised.
        config.providers.entry("custom_archive").reviewed_at = None;
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

    #[tokio::test]
    async fn an_unapproved_enabled_source_is_refused_at_runtime() {
        let mut config = archive_config();
        // A record that expired, or was never completed, must not send traffic
        // just because the name stayed in providers.enabled.
        config.providers.entry("custom_archive").reviewed_at = Some("2020-01-01".into());
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
        assert!(execution.response.providers_used.is_empty());
        let degradation = execution
            .response
            .degradations
            .iter()
            .find(|degradation| degradation.component == "custom_archive")
            .expect("refusal is reported, not silent");
        assert_eq!(degradation.code, "provider_not_approved");
    }

    #[tokio::test]
    async fn an_open_circuit_skips_the_source_until_the_cooldown_expires() {
        let mut config = archive_config();
        config.circuit_breaker.failure_threshold = 1;
        config.circuit_breaker.open_seconds = 600;
        let service = AmatlService::with_registry(
            config,
            false,
            ProviderRegistry::builtin().with(Arc::new(FailingFactory)),
        )
        .await;
        let first = service
            .search("rust".into(), ServiceSurface::cli())
            .await
            .unwrap();
        assert_eq!(first.response.providers_failed, vec!["custom_archive"]);

        let second = service
            .search("rust".into(), ServiceSurface::cli())
            .await
            .unwrap();
        assert!(second.response.providers_failed.is_empty());
        assert!(second
            .response
            .degradations
            .iter()
            .any(|degradation| degradation.code == "provider_circuit_open"));
        assert_eq!(
            service.circuit_snapshots()[0].state,
            crate::CircuitState::Open
        );

        // An operator that fixed the cause can close it without a restart.
        service.reset_circuits().await;
        assert!(service.circuit_snapshots().is_empty());
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
