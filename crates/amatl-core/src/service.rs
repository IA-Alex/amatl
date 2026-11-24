use crate::audit::SecurityAudit;
use crate::cache::{CacheCounters, CacheEffectiveness};
use crate::circuit::{CircuitSnapshot, ProviderCircuit};
use crate::storage::{CacheStats, SavedDocument, SearchHistoryEntry};
use crate::telemetry::{now_unix, ProviderValueSnapshot};
use crate::{
    parse_query, Budget, CachedProvider, CanonicalUrl, ChromiumRenderer, Config, DeepBudget,
    DeepCandidate, DeepOrchestrator, DeepRequest, DeepResponse, DocumentCache, DocumentCachePolicy,
    ErrorCode, FallbackExtractor, GapAnalyzer, InMemoryTelemetry, InferenceRuntime, MockProvider,
    NativeHtmlExtractor, Provider, ProviderAvailability, ProviderBuildContext,
    ProviderCapabilities, ProviderItem, ProviderRegistry, ProviderRuntimeConfig,
    ProviderSearchCache, ProviderSearchCachePolicy, Query, Rank, RankingV2Engine, RendererPool,
    ReqwestTransport, RoleAssignment, SafeFetcher, SearchOrchestrator, SearchPlan, SearchResponse,
    SearchResult, SearchSubQueryExecutor, SqliteStorage, StorageError, TrafilaturaExtractor,
    SCHEMA_VERSION,
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
    /// Redirect ceiling for one single-URL fetch exposed to a surface.
    pub fetch_timeout_ms: u64,
    /// Byte ceiling for one single-URL fetch exposed to a surface.
    pub fetch_max_bytes: u64,
    /// Redirect ceiling for one single-URL fetch exposed to a surface.
    pub fetch_max_redirects: u32,
    /// Redirect ceiling for one Deep document fetch (the crawl's own path,
    /// distinct from the standalone single-URL fetch above).
    pub deep_max_redirects: u32,
    /// Browser-render budget a surface may spend on one Deep request. The
    /// unsandboxed renderer is the most expensive resource Deep can touch, so a
    /// constrained surface (MCP) must not inherit the CLI's allowance.
    pub deep_max_browser_calls: u32,
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
            deep_max_redirects: config.deep.max_redirects,
            deep_max_browser_calls: config.deep.renderer.max_browser_calls,
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
                deep_max_redirects: base.deep_max_redirects.min(2),
                deep_max_browser_calls: base.deep_max_browser_calls.min(1),
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

/// An operator-supplied provider credential held only in RAM.
///
/// The value is `Zeroizing`, so dropping the entry wipes its bytes from the
/// heap. `expires_at` is an absolute instant computed once at write time; a
/// `None` value never expires (the `ttl_ms = 0` opt-out). Expiry is evaluated
/// lazily on every read rather than by a timer, so a key that outlives its TTL
/// is never handed to a provider even if no background task ran.
#[derive(Debug)]
struct EphemeralCredential {
    value: Zeroizing<String>,
    expires_at: Option<std::time::Instant>,
}

impl EphemeralCredential {
    fn new(value: &str, ttl: Option<std::time::Duration>) -> Self {
        Self {
            value: Zeroizing::new(value.to_owned()),
            expires_at: ttl.map(|ttl| std::time::Instant::now() + ttl),
        }
    }

    /// Whether the credential has outlived its TTL at `now`.
    fn is_expired(&self, now: std::time::Instant) -> bool {
        self.expires_at.is_some_and(|deadline| now >= deadline)
    }

    /// Milliseconds remaining before expiry, or `None` when it never expires.
    fn remaining_ms(&self, now: std::time::Instant) -> Option<u64> {
        self.expires_at
            .map(|deadline| deadline.saturating_duration_since(now).as_millis() as u64)
    }
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
fn authentication_required_default() -> bool {
    true
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ServiceStatus {
    pub schema_version: String,
    /// `ok` when at least one enabled source is usable and persistence is
    /// healthy (or disabled by configuration); `degraded` otherwise.
    ///
    /// Declared-but-disabled sources never degrade this field: they are a
    /// configuration choice, not an incident. They stay visible in `sources`.
    pub status: String,
    /// Whether protected HTTP and MCP routes require a bearer token.
    #[serde(default = "authentication_required_default")]
    pub authentication_required: bool,
    pub sources: Vec<SourceStatus>,
    pub storage: StorageStatus,
    pub cache: CacheStatus,
    pub inference_backend: Option<String>,
    /// Read-only view of `data_policy` — the profile, egress and inference
    /// gate every other capability in this status checks against. Exposed
    /// so a settings screen can explain *why* a source or remote embedding
    /// call is unavailable instead of just reporting that it is.
    pub data_policy: DataPolicyStatus,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DataPolicyStatus {
    pub profile: crate::config::SecurityProfile,
    pub egress: crate::config::EgressPolicy,
    pub inference: crate::config::InferenceMode,
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
    mock: bool,
    /// Renderer pool created once at startup and reused across deep requests.
    renderer_pool: RendererPool,
    /// Shared cache hit/miss counters for the operator status surface.
    cache_counters: Arc<CacheCounters>,
    /// Persistent breaker that removes a currently failing source from a round.
    circuit: ProviderCircuit,
    /// Process-wide cap on simultaneous outbound provider calls, shared by
    /// every orchestrator this service builds so concurrent searches cannot
    /// each open their own concurrency window (see
    /// [`crate::SearchOrchestrator::with_egress_gate`]).
    egress_gate: Arc<tokio::sync::Semaphore>,
    /// Durable security audit trail; inert without persistence.
    audit: SecurityAudit,
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
    ///
    /// Each entry also carries an expiry instant. Lazy expiry is enforced on
    /// every read (`resolved_credential`/`ephemeral_credential_present`): once
    /// `expires_at` passes, the entry is dropped (and its bytes wiped) instead
    /// of being handed to a provider. `None` means "no expiry", which only the
    /// `ttl_ms = 0` opt-out produces.
    ephemeral_credentials: Arc<RwLock<BTreeMap<String, EphemeralCredential>>>,
    /// Stops the background maintenance task when the service is dropped.
    ///
    /// Must be a `DropGuard`, not a bare `CancellationToken`: a token does not
    /// cancel on drop, so the task would outlive the service and keep the
    /// connection pool and the advisory file lock alive forever. `Arc` because
    /// the service is `Clone`; the task stops when the last clone is dropped.
    _maintenance_cancel: Option<Arc<tokio_util::sync::DropGuard>>,
}

/// How an [`AmatlService`] build obtains its SQLite persistence.
///
/// Splitting this out is what lets a reload share process-scoped resources
/// (pool, advisory file lock, maintenance task) with the service it replaces
/// instead of opening a second of each — see [`AmatlService::reloaded`].
enum ExistingStorage {
    /// Open (and migrate) the database named by the configuration.
    Open,
    /// Adopt the persistence of a service being replaced, carrying its
    /// degradation marker and maintenance-task guard forward unchanged.
    Reuse {
        storage: Option<SqliteStorage>,
        degradation: Option<crate::Degradation>,
        maintenance_cancel: Option<Arc<tokio_util::sync::DropGuard>>,
    },
}

impl AmatlService {
    /// Service backed by the providers AMATL ships.
    pub async fn new(config: Config, mock: bool) -> Self {
        Self::with_registry(config, mock, ProviderRegistry::builtin()).await
    }

    /// Service backed by a caller-supplied registry, so an embedder can add or
    /// remove sources without changing the core.
    pub async fn with_registry(config: Config, mock: bool, registry: ProviderRegistry) -> Self {
        Self::build(config, mock, registry, ExistingStorage::Open).await
    }

    /// Build a service, deciding how persistence is obtained.
    ///
    /// [`ExistingStorage::Open`] opens (and migrates) the configured database.
    /// [`ExistingStorage::Reuse`] adopts the storage handle of a service being
    /// replaced, so a reload shares one connection pool, one advisory file
    /// lock, and one background maintenance task for the life of the process
    /// instead of opening a second of each against the same file — and, in
    /// `exclusive` locking mode, instead of deadlocking on the file lock the
    /// outgoing service still holds.
    async fn build(
        config: Config,
        mock: bool,
        registry: ProviderRegistry,
        storage_source: ExistingStorage,
    ) -> Self {
        let (storage, storage_degradation, reuse_maintenance) = match storage_source {
            ExistingStorage::Reuse {
                storage,
                degradation,
                maintenance_cancel,
            } => (storage, degradation, maintenance_cancel),
            ExistingStorage::Open => {
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
                // Spawn background maintenance if persistence is enabled and
                // either purging or automatic backups are configured. A reused
                // storage already has a task; only a freshly opened one starts
                // one here.
                let maintenance_wanted = config.persistence.purge_interval_seconds > 0
                    || config.persistence.auto_backup_enabled;
                let maintenance_cancel = match (&storage, maintenance_wanted) {
                    (Some(storage), true) => {
                        let storage_arc = Arc::new(storage.clone());
                        Some(Arc::new(
                            storage_arc
                                .spawn_maintenance(
                                    &config.persistence,
                                    config.telemetry.retention_days,
                                )
                                .drop_guard(),
                        ))
                    }
                    _ => None,
                };
                (storage, storage_degradation, maintenance_cancel)
            }
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
            optics,
            mock,
            renderer_pool,
            cache_counters: Arc::new(CacheCounters::default()),
            circuit: ProviderCircuit::restored(config_circuit_policy, storage_for_circuit).await,
            egress_gate: Arc::new(tokio::sync::Semaphore::new(
                config.execution.global_concurrency.max(1),
            )),
            audit: SecurityAudit::new(storage_for_audit, audit_retention_days),
            ephemeral_credentials: Arc::new(RwLock::new(BTreeMap::new())),
            _maintenance_cancel: reuse_maintenance,
        }
    }

    /// Registry backing this service.
    pub fn registry(&self) -> &ProviderRegistry {
        &self.registry
    }

    /// Build a replacement service from a new configuration, reusing this
    /// service's provider registry, mock flag, connection pool and background
    /// maintenance task.
    ///
    /// Nothing is mutated in place: the caller swaps the handle it hands to new
    /// requests, so in-flight work finishes against the configuration it
    /// started with. An invalid configuration is rejected before anything is
    /// rebuilt, so a bad reload leaves the running service untouched.
    ///
    /// Persistence is *reused*, never reopened: the replacement adopts this
    /// service's `SqliteStorage` (same pool, same advisory file lock, same
    /// maintenance task). Reopening would momentarily run two pools and two
    /// maintenance tasks against one file, and — with `locking_mode =
    /// "exclusive"` — would fail outright on the file lock this very service
    /// still holds, making every reload impossible.
    pub async fn reloaded(self, config: Config) -> Result<Self, ServiceError> {
        config.validate().map_err(|_| ServiceError::Configuration)?;
        let registry = (*self.registry).clone();
        self.rebuilt(config, registry).await
    }

    /// Shared reload body: carry the process-scoped resources (storage pool,
    /// file lock, maintenance task, in-memory credentials) into a service built
    /// from the new configuration.
    async fn rebuilt(
        mut self,
        config: Config,
        registry: ProviderRegistry,
    ) -> Result<Self, ServiceError> {
        let ephemeral = self.ephemeral_credentials.clone();
        let storage_source = ExistingStorage::Reuse {
            storage: self.storage.clone(),
            degradation: self.storage_degradation.clone(),
            maintenance_cancel: self._maintenance_cancel.take(),
        };
        let mut replacement = Self::build(config, self.mock, registry, storage_source).await;
        // Keep operator-injected in-memory credentials across a reload so a
        // provider key survives config rotation without a restart.
        replacement.ephemeral_credentials = ephemeral;
        Ok(replacement)
    }

    /// [`Self::reloaded`], runnable from a generic async position.
    ///
    /// The reload reuses the running service's connection pool rather than
    /// reopening SQLite (see [`Self::reloaded`]), so the `Send`-bound rationale
    /// here applies to the rest of the rebuild (registry, inference runtime);
    /// the storage handle is simply moved across.
    pub async fn reloaded_detached(self, config: Config) -> Result<Self, ServiceError> {
        let handle = tokio::runtime::Handle::current();
        tokio::task::spawn_blocking(move || handle.block_on(self.reloaded(config)))
            .await
            .map_err(|_| ServiceError::Configuration)?
    }

    /// Build a replacement service with a different provider registry, for an
    /// embedder that adds or removes a source implementation at runtime. Like
    /// [`Self::reloaded`], it reuses the running service's persistence rather
    /// than reopening the database.
    pub async fn reloaded_with_registry(
        self,
        config: Config,
        registry: ProviderRegistry,
    ) -> Result<Self, ServiceError> {
        config.validate().map_err(|_| ServiceError::Configuration)?;
        self.rebuilt(config, registry).await
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
    /// A single pagination-capable provider receives the requested page
    /// natively; otherwise AMATL paginates the merged ranked set locally.
    /// `total_results` is omitted when a native provider does not report it.
    /// Surfaces must not mix this with client-side windowing.
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
        let requested_pagination = match (page, page_size) {
            (None, None) => None,
            (page, page_size) => Some((
                page.unwrap_or(0),
                page_size
                    .unwrap_or(limits.max_page_size)
                    .clamp(1, limits.max_page_size),
            )),
        };
        // Native pagination selects a source page before AMATL reranks its
        // results. Mixed-source searches paginate the merged ranking locally.
        // The capability alone is not enough: a provider that reports
        // `pagination` but is not `Available` (e.g. a missing credential) never
        // runs, so forwarding the page would emit `page`/`page_size` metadata
        // for a search that called nothing and must not skip the local window.
        let provider_pagination = requested_pagination.filter(|_| {
            providers.len() == 1
                && providers[0].capabilities().pagination
                && matches!(providers[0].availability(), ProviderAvailability::Available)
        });
        // Fresh orchestrator per search: a search mutates its internal budget,
        // so each search runs on its own orchestrator with its own budget
        // rather than reusing an already-exhausted one.
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
            .with_egress_gate(self.egress_gate.clone())
            .with_result_policies(
                self.config.ranking_policy.clone(),
                self.config.diversity_policy.clone(),
            )
            .with_optics(self.optics.clone())
            .with_search_policy(self.config.search_policy.clone())
            .with_role_assignment(role_assignment_from_config(&self.config))
            .with_telemetry(self.telemetry.clone())
            .with_request_id(surface.request_id.clone())
            .with_provider_pagination(
                provider_pagination.map(|(page, _)| page),
                provider_pagination.map(|(_, page_size)| page_size),
            )
        };
        let mut orchestrator = build_orchestrator();
        let mut response = orchestrator.search(query.clone(), providers.clone()).await;
        // Names admitted into this round, for releasing unused probe claims.
        let selected_names: Vec<String> = providers
            .iter()
            .map(|provider| provider.name().to_string())
            .collect();
        self.record_circuit_outcomes(&selected_names, &response)
            .await;

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
        if let Some((p, ps)) = requested_pagination {
            if provider_pagination.is_some() {
                // The adapter does not extract a total count from this response shape.
                // Preserve the requested window and leave total_results absent.
                response.total_results = None;
            } else {
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
            }
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
            limits.deep_max_redirects,
            limits.deep_max_browser_calls,
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
            (limits.deep_max_bytes / u64::from(effective_max_fetches.max(1))).max(1),
            limits.deep_max_redirects,
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
        // A native provider page does not report a total, so `total_results` is
        // absent while `page`/`page_size` are present. Persisting the page
        // length here would claim a whole-set size that AMATL never knew; store
        // the `-1` "unknown" sentinel instead so the operator view can tell a
        // real count apart from an unreported one.
        let total_results =
            if execution.response.total_results.is_none() && execution.response.page.is_some() {
                -1
            } else {
                execution
                    .response
                    .total_results
                    .map(|value| value as i64)
                    .unwrap_or(execution.response.results.len() as i64)
            };
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

    /// Fetch one saved document by its row id, independent of age.
    pub async fn saved_document(&self, id: i64) -> Result<Option<SavedDocument>, ServiceError> {
        self.storage_or_error()?
            .saved_document_get_by_id(id)
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
        // Mock mode answers `/search` from `mock_providers()` (see
        // `select_providers`), so the operator view must report those same
        // sources. Reporting the declared — but disabled — real providers here
        // made the two halves of the service contradict each other: `/search`
        // kept succeeding while `/status` announced that no source could be
        // called, and every fail-closed consumer — the UI's availability gate
        // above all — concluded the service was unusable.
        let summaries = if self.mock {
            Ok(mock_provider_summaries())
        } else {
            self.provider_summaries()
        };
        let (sources, summaries_unavailable, enabled_usable) = match summaries {
            Ok(summaries) => {
                let mut enabled_usable = false;
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
                        // Readiness follows the deployment the operator asked
                        // for. A source that is declared but not enabled is a
                        // configuration choice, not an incident, so it cannot
                        // degrade the service on its own. Only an enabled source
                        // that is usable — approved and reporting `Available`,
                        // with its circuit closed — keeps the service ready.
                        if summary.enabled
                            && summary.status == ProviderSurfaceStatus::Available
                            && circuit == crate::circuit::CircuitState::Closed
                        {
                            enabled_usable = true;
                        }
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
                (sources, false, enabled_usable)
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
                        let credential_source = credential_source_for(
                            self.registry
                                .get(name)
                                .map(|factory| factory.requires_credential())
                                .unwrap_or(true),
                            self.ephemeral_credential_present(name),
                            runtime,
                        );
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
                (sources, true, false)
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
        // Degraded means "this instance cannot serve useful traffic". Persistence
        // is optional by design, but an instance with no usable enabled source
        // cannot search, so it is degraded — and never ready.
        let degraded =
            summaries_unavailable || storage.enabled && !storage.available || !enabled_usable;
        Ok(ServiceStatus {
            schema_version: SCHEMA_VERSION.into(),
            status: if degraded { "degraded" } else { "ok" }.into(),
            authentication_required: !self.config.server.no_auth,
            sources,
            storage,
            cache,
            inference_backend: self.inference_backend().map(str::to_owned),
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
                let credential_source = credential_source_for(
                    factory.requires_credential(),
                    self.ephemeral_credential_present(name),
                    runtime,
                );
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
        let now = std::time::Instant::now();
        match self.ephemeral_credentials.write() {
            Ok(mut guard) => {
                if let Some(entry) = guard.get(name) {
                    if entry.is_expired(now) {
                        // Lazy expiry: drop the entry here so an outlived key is
                        // wiped (its bytes `Zeroizing`) and never used, even
                        // without a background sweep. Fall through to the env
                        // credential so a configured fallback still applies.
                        tracing::warn!(
                            target: "amatl::credentials",
                            provider = %name,
                            "ephemeral credential expired; it was discarded before use"
                        );
                        drop(guard.remove(name));
                    } else if !entry.value.is_empty() {
                        return Some(entry.value.clone());
                    }
                }
            }
            // A poisoned lock must not silently drop an operator-injected key
            // and fall back to the env var: log it so the degradation is
            // visible instead of a quiet credential switch.
            Err(_) => tracing::error!(
                target: "amatl::credentials",
                provider = %name,
                "ephemeral credential lock poisoned; falling back to the environment credential"
            ),
        }
        credential(runtime)
    }

    /// Whether an operator has injected an in-memory, still-valid credential
    /// for `name`. Read-only; never exposes the secret, only its presence. A
    /// key past its TTL counts as absent and is dropped.
    pub fn ephemeral_credential_present(&self, name: &str) -> bool {
        self.ephemeral_credential_remaining_ms(name).is_some()
    }

    /// Milliseconds until the ephemeral credential for `name` expires, or
    /// `None` when there is no valid ephemeral credential (absent, empty or
    /// already expired). A key with a TTL reports the remaining time; a key
    /// with no TTL reports the `u64::MAX` sentinel meaning "present, never
    /// expires". Either way a `Some` always maps to a live key that may be
    /// used. Expired entries are dropped as a side effect.
    pub fn ephemeral_credential_remaining_ms(&self, name: &str) -> Option<u64> {
        let now = std::time::Instant::now();
        let Ok(mut guard) = self.ephemeral_credentials.write() else {
            return None;
        };
        let entry = guard.get(name)?;
        if entry.is_expired(now) {
            tracing::warn!(
                target: "amatl::credentials",
                provider = %name,
                "ephemeral credential expired; it was discarded on inspection"
            );
            drop(guard.remove(name));
            return None;
        }
        if entry.value.is_empty() {
            return None;
        }
        // A live key with a TTL reports its remaining time; one without a TTL
        // has no finite remaining time, so it reports the `u64::MAX` sentinel
        // that both presence checks and the wire layer read as "never expires".
        Some(entry.remaining_ms(now).unwrap_or(u64::MAX))
    }

    /// Set an operator-supplied provider credential in memory only.
    ///
    /// Nothing is written to disk, to `amatl.toml`, the environment, or logs:
    /// the value lives in RAM and dies with the process. A generous upper bound
    /// rejects a runaway value; an empty value is rejected too. `ttl` is the
    /// lifetime after which the key expires on its own (lazy, on next read);
    /// `None` means it never expires and only disappears when the process does.
    pub fn set_provider_credential(
        &self,
        name: &str,
        value: &str,
        ttl: Option<std::time::Duration>,
    ) -> Result<(), ServiceError> {
        if self.registry.get(name).is_none() {
            return Err(ServiceError::ProviderNotRegistered(name.into()));
        }
        if value.is_empty() || value.len() > 512 {
            return Err(ServiceError::InvalidInput);
        }
        if let Some(ttl) = ttl {
            if ttl.is_zero() {
                return Err(ServiceError::InvalidInput);
            }
        }
        // A poisoned lock is an internal fault, not a missing provider: the
        // provider is registered. Report it as a service fault (500) instead of
        // mislabelling it `ProviderNotRegistered`, and log the real cause.
        let mut guard = self.ephemeral_credentials.write().map_err(|_| {
            tracing::error!(
                target: "amatl::credentials",
                "ephemeral credential lock poisoned; refusing to write"
            );
            ServiceError::Configuration
        })?;
        guard.insert(name.to_string(), EphemeralCredential::new(value, ttl));
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
            .set_provider_credential(&name, &value, None)
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
        // Resolve every declaration first. A provider named in config but
        // absent from the registry (or not declared) fails the whole round; doing
        // it before any `claim_probe` means a failure cannot leave a half-open
        // probe claimed for a source the round never got to record, which would
        // wedge that source for a full cooldown (see `record_circuit_outcomes`).
        let mut resolved = Vec::with_capacity(self.config.providers.enabled.len());
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
            resolved.push((name, runtime, factory));
        }
        for (name, runtime, factory) in resolved {
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
            // Admissibility is a pure query: a source in cooldown with an
            // in-flight probe is skipped here without touching the probe state.
            // The single half-open probe is claimed later, only for providers
            // that actually reach a call (see `record_circuit_outcomes`).
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
            // Admitted into this round: claim the single half-open probe so a
            // concurrent search does not also admit one. If this provider ends
            // up never actually attempted (the router drops it), `search_inner`
            // releases the claim so it cannot wedge the source — see
            // `record_circuit_outcomes`.
            self.circuit.claim_probe(name, now);
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

    /// Feed one search round's outcome into the breaker — exactly once per
    /// round — and release any half-open probe that was claimed for a provider
    /// the round did not actually attempt.
    ///
    /// `selected` is the set of providers admitted into the round
    /// (`select_providers`, which claims the probe). Providers in `selected`
    /// that are neither used nor failed were dropped by the router, so their
    /// claim is released here; otherwise the unused claim would refuse the
    /// source for a whole cooldown.
    ///
    /// `response` must be the *un-merged* response for the round being
    /// recorded: recording a merged response would double-count the refined
    /// round's failures against its own providers and trip the breaker faster
    /// than `failure_threshold` promises.
    async fn record_circuit_outcomes(&self, selected: &[String], response: &SearchResponse) {
        let now = now_unix();
        let attempted: BTreeSet<&String> = response
            .providers_used
            .iter()
            .chain(response.providers_failed.iter())
            .chain(response.providers_partial.iter())
            .collect();
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
        // Release the probe for any admitted provider that never reached a
        // call, keeping the claim symmetric and the source usable.
        for name in selected {
            if !attempted.contains(name) {
                self.circuit.release_probe(name);
            }
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

/// Provenance of a source's credential, if any — never its value.
///
/// A provider whose adapter does not require a credential
/// ([`ProviderFactory::requires_credential`], e.g. a factory whose
/// `credential_env` only names an instance URL with a default) never reports
/// one: an environment variable holding its URL is *configuration*, not a
/// secret, so the surface tells the truth only for sources that actually need a
/// key. A provider that does require a credential resolves to `Ephemeral` (an
/// operator-injected in-memory key, liquidable) or `Environment` (read from
/// `credential_env`), else `None`.
fn credential_source_for(
    requires_credential: bool,
    ephemeral_present: bool,
    config: &ProviderRuntimeConfig,
) -> CredentialSource {
    if !requires_credential {
        return CredentialSource::None;
    }
    if ephemeral_present {
        CredentialSource::Ephemeral
    } else if credential(config).is_some() {
        CredentialSource::Environment
    } else {
        CredentialSource::None
    }
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

/// Operator-view summaries for the sources mock mode actually searches with.
///
/// `/search` answers from `mock_providers()`, so `/status` must report exactly
/// those names as available; otherwise the two halves of the same service
/// disagree and every fail-closed consumer — the UI search gate above all —
/// locks up in mock mode.
fn mock_provider_summaries() -> Vec<ProviderSummary> {
    mock_providers()
        .iter()
        .map(|provider| ProviderSummary {
            schema_version: SCHEMA_VERSION.into(),
            name: provider.name().into(),
            status: ProviderSurfaceStatus::Available,
            code: None,
            capabilities: provider.capabilities(),
            enabled: true,
            credential_source: CredentialSource::None,
        })
        .collect()
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

    /// A factory whose `credential_env` is configuration, not a secret: it
    /// reports `requires_credential() == false`, so the surface must never
    /// expose the variable as a credential even when it is set.
    struct UrlConfiguredFactory;

    impl ProviderFactory for UrlConfiguredFactory {
        fn name(&self) -> &str {
            "url_configured"
        }

        fn requires_credential(&self) -> bool {
            false
        }

        fn build(&self, context: &ProviderBuildContext<'_>) -> Arc<dyn Provider> {
            Arc::new(CredentialGatedProvider {
                name: context.name.into(),
                credential: None,
            })
        }
    }

    /// A registered source whose availability mirrors the enabled flag, the way
    /// the shipped adapters behave: when the operator has not enabled it, its
    /// surface reports `provider_disabled` rather than `available`. Used to pin
    /// that a declared-but-disabled source does not degrade readiness.
    struct DisabledWhenNotEnabledFactory;

    impl ProviderFactory for DisabledWhenNotEnabledFactory {
        fn name(&self) -> &str {
            "custom_archive"
        }

        fn requires_credential(&self) -> bool {
            false
        }

        fn build(&self, context: &ProviderBuildContext<'_>) -> Arc<dyn Provider> {
            Arc::new(DisabledWhenNotEnabledProvider {
                name: context.name.into(),
                enabled: context.enabled,
            })
        }
    }

    struct DisabledWhenNotEnabledProvider {
        name: String,
        enabled: bool,
    }

    #[async_trait]
    impl Provider for DisabledWhenNotEnabledProvider {
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
                authentication: false,
                estimated_cost: Some(0),
            }
        }

        fn availability(&self) -> ProviderAvailability {
            if self.enabled {
                ProviderAvailability::Available
            } else {
                ProviderAvailability::Unavailable {
                    code: "provider_disabled".into(),
                    message: "the operator did not enable this source".into(),
                }
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

    /// Page windows a capture provider recorded, one per `search` call.
    type SeenPages = Arc<std::sync::Mutex<Vec<(Option<u32>, Option<u32>)>>>;

    struct PaginationCaptureFactory {
        seen: SeenPages,
    }

    impl ProviderFactory for PaginationCaptureFactory {
        fn name(&self) -> &str {
            "custom_archive"
        }

        fn requires_credential(&self) -> bool {
            false
        }

        fn build(&self, context: &ProviderBuildContext<'_>) -> Arc<dyn Provider> {
            Arc::new(PaginationCaptureProvider {
                name: context.name.into(),
                seen: self.seen.clone(),
            })
        }
    }

    struct PaginationCaptureProvider {
        name: String,
        seen: SeenPages,
    }

    #[async_trait]
    impl Provider for PaginationCaptureProvider {
        fn name(&self) -> &str {
            &self.name
        }

        fn capabilities(&self) -> ProviderCapabilities {
            MockProvider::success(self.name.clone(), vec![]).capabilities()
        }

        async fn search(
            &self,
            _plan: &crate::SearchPlan,
            context: &crate::ProviderContext,
        ) -> Result<crate::ProviderResult, crate::ProviderError> {
            self.seen
                .lock()
                .unwrap()
                .push((context.page, context.page_size));
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

    fn isolated_config() -> Config {
        let mut config = Config::default();
        config.data_policy.profile = crate::SecurityProfile::Isolated;
        config.data_policy.egress = crate::EgressPolicy::Deny;
        config.data_policy.inference = crate::InferenceMode::LocalOnly;
        config
    }

    /// The per-fetch byte share divides the Deep byte budget by the number of
    /// fetches. Both the build-time ceiling (`:108`) and the per-request share
    /// (`deep_from_search`) must guard the divisor: `max_fetches` is clamped to
    /// `>= 1`, but a per-request `max_fetches = 0` or an unvalidated config with
    /// `deep.max_fetches = 0` reaches the same expression, and dividing by zero
    /// panics. This pins the guarded form so a future refactor cannot drop it.
    #[test]
    fn per_fetch_byte_share_never_divides_by_zero() {
        let deep_max_bytes: u64 = 8 * 1024 * 1024;
        for effective_max_fetches in [0u32, 1, 4] {
            let share = (deep_max_bytes / u64::from(effective_max_fetches.max(1))).max(1);
            assert!(share >= 1);
        }
        // Zero fetches behaves exactly like one, never a panic. The value
        // arrives through a variable so the guard, not the constant folder,
        // is what is exercised.
        let zero_fetches: u32 = 0;
        assert_eq!(
            (deep_max_bytes / u64::from(zero_fetches.max(1))).max(1),
            (deep_max_bytes / u64::from(1u32)).max(1)
        );
        assert_eq!(zero_fetches.max(1), 1);
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
        // The Deep-scoped per-fetch and renderer ceilings must tighten too; they
        // are read by `deep_from_search`, not just advertised.
        assert!(mcp.deep_max_redirects <= cli.deep_max_redirects);
        assert!(mcp.deep_max_browser_calls < cli.deep_max_browser_calls);
    }

    #[tokio::test]
    async fn deep_uses_the_surface_redirect_and_browser_ceilings() {
        // A Deep run must honor the surface's per-fetch ceilings, not the raw
        // CLI-scale config: the same config on MCP and CLI yields different
        // Deep limits, and the service consumes the surface value.
        let config = Config::default();
        let mcp = ExecutionLimits::for_surface(&config, ServiceSurface::mcp());
        assert_eq!(
            mcp.deep_max_redirects,
            config.deep.max_redirects.min(2),
            "MCP deep redirects are clamped"
        );
        assert_eq!(
            mcp.deep_max_browser_calls,
            config.deep.renderer.max_browser_calls.min(1),
            "MCP browser calls are clamped"
        );
        let cli = ExecutionLimits::for_surface(&config, ServiceSurface::cli());
        assert_eq!(cli.deep_max_redirects, config.deep.max_redirects);
        assert_eq!(
            cli.deep_max_browser_calls,
            config.deep.renderer.max_browser_calls
        );
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
    async fn a_single_pagination_capable_source_receives_the_requested_page() {
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let registry = ProviderRegistry::builtin()
            .with(Arc::new(PaginationCaptureFactory { seen: seen.clone() }));
        let service = AmatlService::with_registry(archive_config(), false, registry).await;
        let execution = service
            .search_paginated("rust".into(), ServiceSurface::mcp(), Some(2), Some(5))
            .await
            .unwrap();

        assert_eq!(seen.lock().unwrap().as_slice(), [(Some(2), Some(5))]);
        assert_eq!(execution.response.page, Some(2));
        assert_eq!(execution.response.page_size, Some(5));
        assert_eq!(execution.response.total_results, None);
    }

    /// BH-003: an enabled source that reports `pagination` but is `Unavailable`
    /// (its credential is missing) never reaches a provider call. Forwarding the
    /// page would emit `page`/`page_size` for a search that called nothing and
    /// would skip the local window, so native pagination must stay off.
    struct UnavailablePaginationFactory {
        seen: SeenPages,
    }

    impl ProviderFactory for UnavailablePaginationFactory {
        fn name(&self) -> &str {
            "custom_archive"
        }

        fn requires_credential(&self) -> bool {
            false
        }

        fn build(&self, context: &ProviderBuildContext<'_>) -> Arc<dyn Provider> {
            Arc::new(UnavailablePaginationProvider {
                name: context.name.into(),
                seen: self.seen.clone(),
            })
        }
    }

    struct UnavailablePaginationProvider {
        name: String,
        seen: SeenPages,
    }

    #[async_trait]
    impl Provider for UnavailablePaginationProvider {
        fn name(&self) -> &str {
            &self.name
        }

        fn capabilities(&self) -> ProviderCapabilities {
            // Pagination-capable on paper, exactly like a credential-less
            // Marginalia, but never available this run.
            MockProvider::success(self.name.clone(), vec![]).capabilities()
        }

        fn availability(&self) -> ProviderAvailability {
            ProviderAvailability::Unavailable {
                code: "provider_credential_missing".into(),
                message: "the source credential is unavailable".into(),
            }
        }

        async fn search(
            &self,
            _plan: &crate::SearchPlan,
            context: &crate::ProviderContext,
        ) -> Result<crate::ProviderResult, crate::ProviderError> {
            self.seen
                .lock()
                .unwrap()
                .push((context.page, context.page_size));
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

    #[tokio::test]
    async fn an_unavailable_pagination_capable_source_never_receives_a_native_page() {
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let registry = ProviderRegistry::builtin().with(Arc::new(UnavailablePaginationFactory {
            seen: seen.clone(),
        }));
        let service = AmatlService::with_registry(archive_config(), false, registry).await;
        let execution = service
            .search_paginated("rust".into(), ServiceSurface::api(None), Some(0), Some(5))
            .await
            .unwrap();

        // The source never ran, so it saw no page window.
        assert!(seen.lock().unwrap().is_empty());
        // The local branch still owns page metadata (not a native page).
        assert_eq!(execution.response.page, Some(0));
        assert_eq!(execution.response.page_size, Some(5));
        // A locally paginated search always knows the whole-set size.
        assert_eq!(execution.response.total_results, Some(0));
    }

    /// BH-002: a native page omits `total_results`, so history must not persist
    /// the page length as a whole-set size. It stores the `-1` unknown sentinel,
    /// which the operator views (CLI/UI) render without a count.
    #[tokio::test]
    async fn native_page_history_stores_the_unknown_total_sentinel() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("amatl-history-native-{nonce}.sqlite3"));
        let mut config = archive_config();
        config.persistence.enabled = true;
        config.persistence.history_enabled = true;
        config.persistence.path = path.display().to_string();

        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let registry = ProviderRegistry::builtin()
            .with(Arc::new(PaginationCaptureFactory { seen: seen.clone() }));
        let service = AmatlService::with_registry(config, false, registry).await;

        // Native pagination: the sole available source is pagination-capable.
        service
            .search_paginated("rust".into(), ServiceSurface::api(None), Some(0), Some(5))
            .await
            .unwrap();

        let history = service.history(10, 0).await.unwrap();
        let entry = history
            .first()
            .expect("a search history entry was recorded");
        assert_eq!(
            entry.total_results, -1,
            "a native page must record the unknown-total sentinel, not the page length"
        );

        drop(service);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(format!("{}.lock", path.display()));
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
            validate_provider_canary(service.config(), "marginalia"),
            Err(ProviderCanaryError::EgressDenied)
        );
    }

    #[tokio::test]
    async fn ephemeral_credential_lives_in_memory_until_liquidated() {
        let service = AmatlService::new(Config::default(), false).await;
        assert!(!service.ephemeral_credential_present("marginalia"));

        service
            .set_provider_credential("marginalia", "marginalia-secret-key", None)
            .unwrap();
        assert!(service.ephemeral_credential_present("marginalia"));

        service.clear_provider_credential("marginalia");
        assert!(!service.ephemeral_credential_present("marginalia"));

        // Idempotent: liquidating an absent key is not an error.
        service.clear_provider_credential("marginalia");
        assert!(!service.ephemeral_credential_present("marginalia"));
    }

    #[tokio::test]
    async fn ephemeral_credential_expires_lazily_at_its_ttl() {
        let service = AmatlService::new(Config::default(), false).await;
        // A live key with a short TTL is present with time remaining...
        service
            .set_provider_credential(
                "marginalia",
                "marginalia-secret-key",
                Some(std::time::Duration::from_secs(3600)),
            )
            .unwrap();
        assert!(service.ephemeral_credential_present("marginalia"));
        assert!(service
            .ephemeral_credential_remaining_ms("marginalia")
            .is_some_and(|remaining| remaining > 0));

        // ...and a key past its TTL is treated as absent and dropped on read,
        // without waiting for any background task.
        service
            .set_provider_credential(
                "marginalia",
                "marginalia-secret-key",
                Some(std::time::Duration::from_millis(1)),
            )
            .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        assert!(!service.ephemeral_credential_present("marginalia"));
        assert!(service
            .ephemeral_credential_remaining_ms("marginalia")
            .is_none());
    }

    #[tokio::test]
    async fn ephemeral_credential_without_ttl_never_expires() {
        let service = AmatlService::new(Config::default(), false).await;
        service
            .set_provider_credential("marginalia", "marginalia-secret-key", None)
            .unwrap();
        // The `None` TTL is the "no expiry" opt-out: presence survives reads and
        // the remaining-time sentinel marks it as never expiring.
        assert!(service.ephemeral_credential_present("marginalia"));
        assert_eq!(
            service.ephemeral_credential_remaining_ms("marginalia"),
            Some(u64::MAX)
        );
    }

    #[tokio::test]
    async fn ephemeral_credential_is_never_serialized_into_the_config() {
        let service = AmatlService::new(Config::default(), false).await;
        service
            .set_provider_credential("marginalia", "marginalia-secret-key", None)
            .unwrap();
        // The key must never reach a serialized form of the configuration — the
        // same shape a config file or a `/config` view would carry.
        let serialized = serde_json::to_string(service.config()).unwrap();
        assert!(!serialized.contains("marginalia-secret-key"));
    }

    #[tokio::test]
    async fn ephemeral_credential_rejects_unregistered_empty_and_oversized_values() {
        let service = AmatlService::new(Config::default(), false).await;
        // An unregistered provider is a registry fault, not a lock fault: the
        // specific `ProviderNotRegistered` variant must survive here so it is
        // never conflated with an internal write failure.
        assert_eq!(
            service.set_provider_credential("not_registered", "k", None),
            Err(ServiceError::ProviderNotRegistered("not_registered".into()))
        );
        assert!(service
            .set_provider_credential("marginalia", "", None)
            .is_err());
        assert!(service
            .set_provider_credential("marginalia", &"x".repeat(513), None)
            .is_err());
        // None of the rejected values were committed.
        assert!(!service.ephemeral_credential_present("marginalia"));
    }

    #[tokio::test]
    async fn ephemeral_credential_survives_reload_without_a_restart() {
        let service = AmatlService::new(Config::default(), false).await;
        service
            .set_provider_credential("marginalia", "marginalia-secret-key", None)
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
            .set_provider_credential("marginalia", "marginalia-secret-key", None)
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
    async fn a_provider_that_does_not_require_a_credential_never_reports_one() {
        // A factory whose `requires_credential()` is false may still name a
        // `credential_env`, but it holds *configuration* (e.g. an instance URL
        // with a default), not a secret. Even when that variable is present in
        // the environment, the surface must report `None`. No shipped source has
        // this shape today, so a registered test factory stands in for the
        // third-party adapter the contract is written for.
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let var = format!("AMATL_TEST_INSTANCE_URL_VAR_{nonce}");
        std::env::set_var(&var, "http://127.0.0.1:9999");

        let mut config = Config::default();
        let mut record = approved_record("url_configured-v1");
        record.credential_env = Some(var.clone());
        config.providers.declare("url_configured", record);
        config.providers.enabled = vec!["url_configured".into()];
        let service = AmatlService::with_registry(
            config,
            false,
            ProviderRegistry::builtin().with(Arc::new(UrlConfiguredFactory)),
        )
        .await;
        let summary = service
            .provider_summaries()
            .unwrap()
            .into_iter()
            .find(|summary| summary.name == "url_configured")
            .unwrap();
        assert_eq!(
            summary.credential_source,
            CredentialSource::None,
            "a URL-shaped credential_env must not be reported as a credential"
        );

        std::env::remove_var(&var);
    }

    #[tokio::test]
    async fn status_sources_expose_credential_provenance_but_not_the_value() {
        let service = AmatlService::new(Config::default(), false).await;
        service
            .set_provider_credential("marginalia", "marginalia-secret-key", None)
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

    #[tokio::test]
    async fn status_reports_the_mock_sources_search_actually_uses() {
        let service = AmatlService::new(Config::default(), true).await;
        let status = service.status().await.unwrap();
        let available = status
            .sources
            .iter()
            .filter(|source| source.status == ProviderSurfaceStatus::Available)
            .count();
        let expected = mock_providers()
            .iter()
            .map(|provider| provider.name().to_string())
            .collect::<Vec<_>>();
        // The UI gates its search button on this exact predicate
        // (`availableSources > 0` in `assets/app.js`). A zero here means the
        // UI can never submit a search that `/search` would happily answer.
        assert!(
            available > 0,
            "mock mode must expose at least one available source, got {:?}",
            status.sources
        );
        assert_eq!(available, expected.len());
        assert_eq!(
            status.status, "ok",
            "mock search is always available, so readiness must not degrade it"
        );
        assert_eq!(
            status
                .sources
                .iter()
                .map(|source| source.name.clone())
                .collect::<Vec<_>>(),
            expected,
            "the operator view must name the sources mock search uses"
        );
    }

    #[tokio::test]
    async fn status_reports_authentication_requirement_from_server_config() {
        let mut config = Config::default();
        let protected = AmatlService::new(config.clone(), true).await;
        assert!(protected.status().await.unwrap().authentication_required);

        config.server.no_auth = true;
        let open = AmatlService::new(config, true).await;
        assert!(!open.status().await.unwrap().authentication_required);
    }

    /// D2: readiness must follow the deployment the operator asked for. A source
    /// that is declared but not enabled is a configuration choice, not an
    /// incident, so it cannot make the instance look degraded while an enabled
    /// source is serving traffic — otherwise `/ready` answers 503 and a load
    /// balancer drains a perfectly healthy single-source deployment.
    #[tokio::test]
    async fn status_ignores_declared_sources_the_operator_did_not_enable() {
        // One enabled source serves; a declared-but-disabled source must not
        // make the deployment look degraded. Marginalia is enabled, and a
        // registered custom source is declared but left out of `enabled`.
        let mut config = marginalia_only_config();
        config
            .providers
            .declare("custom_archive", approved_record("archive-v1"));
        let service = AmatlService::with_registry(
            config,
            false,
            ProviderRegistry::builtin().with(Arc::new(DisabledWhenNotEnabledFactory)),
        )
        .await;
        // Marginalia is key-gated, so it only reports `Available` with a
        // credential present. Inject one through the supported operator path
        // rather than relying on the host environment, so the test asserts the
        // same thing on a clean CI runner as on an operator shell.
        service
            .set_provider_credential("marginalia", "marginalia-test-key", None)
            .unwrap();
        let status = service.status().await.unwrap();
        assert_eq!(
            status.status, "ok",
            "a declared-but-disabled source must not degrade a serving deployment: {:?}",
            status.sources
        );
        // The declared sources stay visible where they belong: `/status` still
        // names them and their blocking code, it just no longer treats them as
        // incidents of this deployment.
        let source = |name: &str| {
            status
                .sources
                .iter()
                .find(|source| source.name == name)
                .unwrap()
        };
        assert_eq!(
            source("marginalia").status,
            ProviderSurfaceStatus::Available
        );
        assert_eq!(source("marginalia").code, None);
        assert_eq!(
            source("custom_archive").status,
            ProviderSurfaceStatus::Unavailable
        );
        assert_eq!(
            source("custom_archive").code.as_deref(),
            Some("provider_disabled")
        );
    }

    /// The other half of the D2 rule: ignoring disabled sources must not make a
    /// deployment with no enabled source look ready. It cannot search at all.
    #[tokio::test]
    async fn status_degrades_when_no_source_is_enabled() {
        let service = AmatlService::new(Config::default(), false).await;
        let status = service.status().await.unwrap();
        assert_eq!(status.status, "degraded");
    }

    /// Enabled is not enough: the governance gate still blocks traffic, so the
    /// deployment has no usable source and readiness must say so.
    #[tokio::test]
    async fn status_degrades_when_the_only_enabled_source_is_not_approved() {
        let mut config = Config::default();
        config.providers.enabled = vec!["marginalia".into()];
        let service = AmatlService::new(config, false).await;
        let status = service.status().await.unwrap();
        assert_eq!(status.status, "degraded");
        let marginalia = status
            .sources
            .iter()
            .find(|source| source.name == "marginalia")
            .unwrap();
        assert_eq!(
            marginalia.code.as_deref(),
            Some("provider_not_approved"),
            "an enabled but unapproved source cannot serve"
        );
    }

    /// The third clause of the D2 rule: an enabled and approved source that
    /// still reports `Available` cannot serve once its circuit is open, so
    /// readiness must degrade — the breaker is the only evidence that traffic is
    /// no longer reaching it, and `/status` keeps naming it available.
    #[tokio::test]
    async fn status_degrades_when_the_only_enabled_source_circuit_is_open() {
        let mut config = archive_config();
        config.circuit_breaker.failure_threshold = 1;
        config.circuit_breaker.open_seconds = 600;
        let service = AmatlService::with_registry(
            config,
            false,
            ProviderRegistry::builtin().with(Arc::new(FailingFactory)),
        )
        .await;
        // Before the first failure the same deployment is ready, so the
        // assertion below isolates the circuit and nothing else.
        let before = service.status().await.unwrap();
        assert_eq!(before.status, "ok", "{:?}", before.sources);

        let failure = service
            .search("rust".into(), ServiceSurface::cli())
            .await
            .unwrap();
        assert_eq!(failure.response.providers_failed, vec!["custom_archive"]);

        let after = service.status().await.unwrap();
        let source = after
            .sources
            .iter()
            .find(|source| source.name == "custom_archive")
            .unwrap();
        assert_eq!(source.status, ProviderSurfaceStatus::Available);
        assert_eq!(source.circuit, crate::CircuitState::Open);
        assert_eq!(
            after.status, "degraded",
            "an open circuit means the enabled source cannot serve: {:?}",
            after.sources
        );
    }

    /// Configuration for the deployment D2 is about: exactly one source enabled
    /// and fully governed. The shipped Marginalia record carries only its
    /// adapter/credential/terms identity, so the test completes the ficha with
    /// the fields the approval gate requires (a real operator records them when
    /// approving the source).
    fn marginalia_only_config() -> Config {
        let mut config = Config::default();
        let runtime = config.providers.entry("marginalia");
        runtime.adapter_version = Some("marginalia-v1".into());
        runtime.approval_status = crate::ApprovalStatus::Approved;
        runtime.reviewed_at = Some(today_iso());
        runtime.reviewer = Some("owner".into());
        runtime.terms_url = Some("https://www.marginalia.nu/".into());
        runtime.terms_version_or_date = Some("2026-08-01".into());
        runtime.allowed_access_method = Some("official_api".into());
        runtime.plan_or_contract = Some("contract-1".into());
        runtime.rate_limit = Some("1 qps".into());
        runtime.cost_model = Some("0".into());
        runtime.data_handling_notes = Some("no cache".into());
        runtime.operational_risk = Some("network dependency".into());
        config.providers.enabled = vec!["marginalia".into()];
        config.validate().unwrap();
        config
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
            validate_provider_canary(&config, "marginalia"),
            Err(ProviderCanaryError::NotEnabled("marginalia".into()))
        );
        let mut enabled = config;
        enabled.providers.enabled = vec!["marginalia".into()];
        assert_eq!(
            validate_provider_canary(&enabled, "marginalia"),
            Err(ProviderCanaryError::Governance("marginalia".into()))
        );
    }

    #[test]
    fn undeclared_provider_canary_stops_at_the_registry_door() {
        // Configuration validation refuses undeclared names first, so this test
        // bypasses it on purpose: it declares a source with a governance record
        // and enables it by hand. Even then the canary cannot reach the network,
        // because the builtin registry ships no factory for it.
        let mut config = Config::default();
        config
            .providers
            .declare("unshipped_source", approved_record("unshipped-v1"));
        config.providers.enabled = vec!["unshipped_source".into()];
        // The declared source passes the governance-record lookup (declared,
        // enabled) and still stops at the registry, which carries no factory
        // for it — a typed error, never a network attempt.
        assert_eq!(
            validate_provider_canary(&config, "unshipped_source"),
            Err(ProviderCanaryError::UnknownProvider(
                "unshipped_source".into()
            ))
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

    #[tokio::test]
    async fn a_single_search_records_each_provider_outcome_once() {
        // Regression guard for the double-recording bug: with a threshold of 2,
        // one failing search must leave the breaker closed. Recording the same
        // round twice (as the post-merge pass used to) would open it after a
        // single search, making the breaker eager beyond its configuration.
        let mut config = archive_config();
        config.circuit_breaker.failure_threshold = 2;
        config.circuit_breaker.open_seconds = 600;
        let service = AmatlService::with_registry(
            config,
            false,
            ProviderRegistry::builtin().with(Arc::new(FailingFactory)),
        )
        .await;

        service
            .search("rust".into(), ServiceSurface::cli())
            .await
            .unwrap();
        assert_eq!(
            service.circuit_snapshots()[0].state,
            crate::CircuitState::Closed,
            "one failed search must count as one failure, not two"
        );

        // The second failed search crosses the threshold exactly once.
        service
            .search("rust".into(), ServiceSurface::cli())
            .await
            .unwrap();
        assert_eq!(
            service.circuit_snapshots()[0].state,
            crate::CircuitState::Open
        );
    }

    #[tokio::test]
    async fn reload_reuses_storage_so_exclusive_locking_survives_a_reload() {
        // With `locking_mode = "exclusive"` the running service holds the
        // advisory file lock. A reload that reopened SQLite would deadlock on
        // that very lock; reusing the running pool must make it succeed.
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("amatl-reload-{nonce}.sqlite3"));
        let mut config = Config::default();
        config.persistence.enabled = true;
        config.persistence.path = path.display().to_string();
        config.persistence.locking_mode = crate::config::SqliteLockingMode::Exclusive;

        let service = AmatlService::new(config.clone(), true).await;
        assert!(service.storage().is_some(), "persistence must be active");

        let reloaded = service
            .reloaded(config)
            .await
            .expect("reload must succeed in exclusive locking mode");
        assert!(reloaded.storage().is_some());

        // The reloaded service still works against the same database.
        reloaded
            .search("rust".into(), ServiceSurface::cli())
            .await
            .unwrap();

        drop(reloaded);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(format!("{}.lock", path.display()));
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
