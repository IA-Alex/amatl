use crate::diversity::DiversityPolicyV1;
use crate::gaps::GapPolicyV1;
use crate::progressive::SearchPolicyV1;
use crate::ranking::RankingPolicyV1;
use crate::ranking_v2::RankingV2Policy;
use serde::{Deserialize, Serialize};
use std::path::Path;
use thiserror::Error;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Config {
    /// Schema version this configuration file was written for. Must match the
    /// running binary's [`crate::SCHEMA_VERSION`]; a mismatch is rejected at
    /// startup so an operator never runs with silently-ignored fields.
    #[serde(default = "default_schema_version")]
    pub schema_version: String,
    pub data_policy: DataPolicyConfig,
    pub inference: InferenceConfig,
    pub providers: ProviderConfig,
    pub timeouts: TimeoutConfig,
    pub budget: BudgetConfig,
    pub execution: ExecutionConfig,
    pub ranking_policy: RankingPolicyV1,
    pub diversity_policy: DiversityPolicyV1,
    pub search_policy: SearchPolicyV1,
    pub persistence: PersistenceConfig,
    /// Trip limits for the persistent provider circuit breaker.
    pub circuit_breaker: crate::circuit::CircuitPolicy,
    pub cache: CacheConfig,
    pub telemetry: TelemetryConfig,
    pub deep: DeepConfig,
    pub server: ServerConfig,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct DataPolicyConfig {
    pub profile: SecurityProfile,
    pub egress: EgressPolicy,
    pub inference: InferenceMode,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SecurityProfile {
    #[default]
    Standard,
    Isolated,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EgressPolicy {
    Deny,
    #[default]
    Governed,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InferenceMode {
    #[default]
    Disabled,
    LocalOnly,
    RemoteExplicit,
}

impl DataPolicyConfig {
    pub fn allows_network_egress(&self) -> bool {
        self.profile != SecurityProfile::Isolated && self.egress == EgressPolicy::Governed
    }

    pub fn allows_local_inference(&self) -> bool {
        self.inference != InferenceMode::Disabled
    }

    pub fn allows_remote_inference(&self) -> bool {
        self.profile != SecurityProfile::Isolated
            && self.egress == EgressPolicy::Governed
            && self.inference == InferenceMode::RemoteExplicit
    }
}

impl SecurityProfile {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::Isolated => "isolated",
        }
    }
}

impl EgressPolicy {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Deny => "deny",
            Self::Governed => "governed",
        }
    }
}

impl InferenceMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::LocalOnly => "local_only",
            Self::RemoteExplicit => "remote_explicit",
        }
    }
}

/// Backend limits for the optional inference layer.
///
/// The mode itself lives in `data_policy.inference`; this section only sizes
/// the backend that mode selects. See [`crate::inference`].
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct InferenceConfig {
    /// Backend identifier used by `local_only`.
    pub backend: String,
    /// Width of the embedding vectors.
    pub embedding_dimensions: usize,
    /// Maximum documents scored in one ranking call.
    pub max_documents: usize,
    /// Maximum characters read per document before embedding.
    pub max_input_chars: usize,
    /// Weight the reranker keeps for the upstream relevance signal.
    pub reranker_prior_weight: f64,
    /// Path to a local word-vector model file used by the `local_model_v1`
    /// backend. Each line is `token v0 v1 … vN` (space separated floats).
    /// When unset, `local_model_v1` fails closed and Deep degrades to the
    /// hashing backend.
    pub local_model_path: Option<String>,
    /// Sizing hint for the on-disk embedding cache of `local_model_v1`.
    ///
    /// Despite the name this does **not** cap how many documents are embedded
    /// per call: nothing in the pipeline batches on it. Its only effect is the
    /// capacity of [`crate::inference::EmbeddingCache`] (this value times 64
    /// entries). The name is kept for rc.1 to avoid breaking existing
    /// configuration files; it should be renamed to `local_cache_capacity`
    /// before 1.0.
    pub local_model_batch: usize,
    /// Optional path where computed embeddings are cached between executions.
    /// The cache is namespaced by the vector-space identity so artifacts are
    /// never reused across backends or widths. When unset, no cache is used.
    pub local_cache_path: Option<String>,
    /// Embeddings endpoint used by `data_policy.inference = "remote_explicit"`.
    /// Must be absolute HTTPS, or HTTP on loopback for a self-hosted server,
    /// and must not embed credentials.
    pub remote_endpoint: Option<String>,
    /// Model identifier sent in the remote request body.
    pub remote_model: Option<String>,
    /// Environment variable holding the bearer credential, when the endpoint
    /// requires one. The value is never written to configuration or logs.
    pub remote_credential_env: Option<String>,
    /// Deadline for one remote embeddings request.
    pub remote_timeout_ms: u64,
    /// Largest number of inputs sent in a single remote request.
    pub remote_max_batch: usize,
}

impl Default for InferenceConfig {
    fn default() -> Self {
        Self {
            backend: crate::inference::LOCAL_EMBEDDING_BACKEND_ID.into(),
            embedding_dimensions: 256,
            max_documents: 64,
            max_input_chars: 20_000,
            reranker_prior_weight: 0.5,
            local_model_path: None,
            local_model_batch: 32,
            local_cache_path: None,
            remote_endpoint: None,
            remote_model: None,
            remote_credential_env: None,
            remote_timeout_ms: 5_000,
            remote_max_batch: 32,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ServerConfig {
    pub bind: String,
    pub port: u16,
    /// Environment variable holding the single default credential.
    ///
    /// Kept for deployments with one operator: the resulting client is called
    /// `default` and carries every scope and every MCP tool. Declare
    /// `[[server.clients]]` instead to split capability between callers.
    pub token_env: String,
    pub no_auth: bool,
    /// Named credentials, each with its own scopes, MCP tool allowlist and
    /// optional expiry. A request is authorized as exactly one of them.
    pub clients: Vec<ServerClient>,
    pub allowed_hosts: Vec<String>,
    pub allowed_origins: Vec<String>,
    pub max_body_bytes: usize,
    pub max_header_bytes: usize,
    pub request_timeout_ms: u64,
    pub idle_timeout_ms: u64,
    pub rate_limit_per_minute: u32,
    pub max_connections: usize,
    pub tls: TlsConfig,
}

/// One named credential accepted by the HTTP and MCP surfaces.
///
/// The secret never lives in configuration: declare the environment variable
/// that holds it, or the SHA-256 digest of the token. Both forms are compared
/// as digests in constant time, so a configuration file leak does not leak a
/// usable credential.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ServerClient {
    /// Stable identity recorded in audit events. Never a secret.
    pub id: String,
    /// Environment variable holding this client's bearer token.
    pub token_env: Option<String>,
    /// Lowercase hex SHA-256 digest of this client's bearer token.
    pub token_sha256: Option<String>,
    /// ISO `YYYY-MM-DD` after which the credential stops being accepted.
    pub expires_at: Option<String>,
    /// HTTP capability granted to this client.
    pub scopes: Vec<Scope>,
    /// MCP tools this client may call. Empty means no MCP access at all: tool
    /// capability is granted explicitly, never inherited.
    pub tools: Vec<String>,
}

/// Capability a route requires from the calling client.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    /// `/search`.
    Search,
    /// `/deep`.
    Deep,
    /// Read-only operator surfaces: `/providers`, `/status`, and listing
    /// history and saved documents.
    Read,
    /// Mutating local state: saving documents, deleting or purging history.
    Write,
    /// `/reload` and any other administrative action.
    Admin,
    /// The MCP surface itself. The per-tool allowlist narrows it further.
    Mcp,
}

impl Scope {
    pub const ALL: [Scope; 6] = [
        Scope::Search,
        Scope::Deep,
        Scope::Read,
        Scope::Write,
        Scope::Admin,
        Scope::Mcp,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Search => "search",
            Self::Deep => "deep",
            Self::Read => "read",
            Self::Write => "write",
            Self::Admin => "admin",
            Self::Mcp => "mcp",
        }
    }
}

impl ServerClient {
    /// Whether the credential is still valid on `today` (`YYYY-MM-DD`).
    pub fn unexpired_on(&self, today: &str) -> bool {
        match self.expires_at.as_deref() {
            None => true,
            Some(expiry) => match (day_from_iso_date(expiry), day_from_iso_date(today)) {
                (Some(expiry), Some(today)) => today <= expiry,
                // An unparseable expiry fails closed.
                _ => false,
            },
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct TlsConfig {
    pub cert_path: Option<String>,
    pub key_path: Option<String>,
}

/// Declared search sources.
///
/// Every `[providers.<name>]` table is a governance record keyed by provider
/// name, so a new source is declared without changing this struct. The name
/// must also exist in the [`crate::ProviderRegistry`] for the service to build
/// it; configuration declares, the registry implements.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(from = "ProviderConfigWire")]
pub struct ProviderConfig {
    pub enabled: Vec<String>,
    #[serde(flatten)]
    declared: std::collections::BTreeMap<String, ProviderRuntimeConfig>,
}

/// Wire shape of `[providers]`: the fixed `enabled` list plus one table per
/// declared source. Declarations merge over the built-in records so a file that
/// tunes one provider does not drop the others.
#[derive(Default, Deserialize)]
#[serde(default)]
struct ProviderConfigWire {
    enabled: Vec<String>,
    #[serde(flatten)]
    declared: std::collections::BTreeMap<String, ProviderRuntimeConfig>,
}

impl From<ProviderConfigWire> for ProviderConfig {
    fn from(wire: ProviderConfigWire) -> Self {
        let mut declared = builtin_provider_records();
        declared.extend(wire.declared);
        Self {
            enabled: wire.enabled,
            declared,
        }
    }
}

impl ProviderConfig {
    /// Governance record for a provider name, if it is declared.
    pub fn get(&self, name: &str) -> Option<&ProviderRuntimeConfig> {
        self.declared.get(name)
    }

    /// Mutable governance record, creating a default one when absent.
    pub fn entry(&mut self, name: impl Into<String>) -> &mut ProviderRuntimeConfig {
        self.declared.entry(name.into()).or_default()
    }

    /// Declare or replace a governance record.
    pub fn declare(&mut self, name: impl Into<String>, runtime: ProviderRuntimeConfig) {
        self.declared.insert(name.into(), runtime);
    }

    /// Remove a declaration, returning the record it held.
    pub fn remove(&mut self, name: &str) -> Option<ProviderRuntimeConfig> {
        self.declared.remove(name)
    }

    pub fn is_declared(&self, name: &str) -> bool {
        self.declared.contains_key(name)
    }

    /// All declarations in stable alphabetical order.
    pub fn declared(&self) -> impl Iterator<Item = (&String, &ProviderRuntimeConfig)> {
        self.declared.iter()
    }

    /// Declared provider names in stable alphabetical order.
    pub fn names(&self) -> Vec<&str> {
        self.declared.keys().map(String::as_str).collect()
    }

    /// Whether the source may keep retrieved content, defaulting to `false`
    /// for anything undeclared.
    pub fn storage_rights(&self, name: &str) -> bool {
        self.get(name).is_some_and(|runtime| runtime.storage_rights)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ProviderRuntimeConfig {
    pub adapter_version: Option<String>,
    pub approval_status: ApprovalStatus,
    pub reviewed_at: Option<String>,
    pub reviewer: Option<String>,
    pub terms_url: Option<String>,
    pub terms_version_or_date: Option<String>,
    pub allowed_access_method: Option<String>,
    pub plan_or_contract: Option<String>,
    pub rate_limit: Option<String>,
    pub cost_model: Option<String>,
    pub credential_env: Option<String>,
    pub storage_rights: bool,
    pub supported_regions: Vec<String>,
    pub supported_filters: Vec<String>,
    pub data_handling_notes: Option<String>,
    pub operational_risk: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalStatus {
    #[default]
    Draft,
    Approved,
    Expired,
    Rejected,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct TimeoutConfig {
    pub provider_ms: u64,
    pub global_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct BudgetConfig {
    pub max_provider_calls: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ExecutionConfig {
    pub global_concurrency: usize,
    pub per_provider_concurrency: usize,
    pub max_retries: u32,
    pub retry_jitter_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct PersistenceConfig {
    pub enabled: bool,
    pub path: String,
    /// Record every executed search in the local history table. Requires
    /// `enabled`; history never leaves the machine.
    pub history_enabled: bool,
    /// Upper bound on the payload a surface may persist as a saved document.
    pub saved_document_max_bytes: u64,
    /// Days a persisted security event is kept. Requires `enabled`.
    pub audit_retention_days: u32,
    /// Days search history entries are kept (0 = unlimited). Requires `enabled`.
    pub history_retention_days: u32,
    /// Days provider search cache entries are kept (0 = unlimited). Requires `enabled`.
    pub cache_retention_days: u32,
    /// Days document cache entries are kept (0 = unlimited). Requires `enabled`.
    pub document_cache_retention_days: u32,
    /// Seconds between scheduled purge cycles (0 = disabled). Requires `enabled`.
    pub purge_interval_seconds: u64,
    /// Whether to create automatic periodic backups. Requires `enabled`.
    pub auto_backup_enabled: bool,
    /// Seconds between automatic backups (86400 = daily). Requires `auto_backup_enabled`.
    pub auto_backup_interval_seconds: u64,
    /// Maximum number of automatic backups to retain (oldest rotated first).
    pub auto_backup_max_count: u32,
    /// Directory for automatic backups (default: same directory as the database).
    pub backup_directory: Option<String>,
    /// Cross-process locking discipline: "normal" (default) or "exclusive".
    pub locking_mode: SqliteLockingMode,
}

/// How AMATL processes coordinate access to one database file.
///
/// This does **not** map to SQLite's `PRAGMA locking_mode`. That pragma would
/// lock out every reader, including `amatl db health` and the `sqlite3` CLI,
/// and is incompatible with keeping WAL mode usable. What is taken instead is
/// an advisory `flock` on a sibling `.lock` file.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SqliteLockingMode {
    /// Default WAL mode: concurrent readers, one writer.
    Normal,
    /// Advisory exclusive lock between AMATL processes.
    ///
    /// Cooperative by nature: it stops a second `amatl` from opening the
    /// database, but any other tool that does not check the lock file is
    /// unaffected.
    Exclusive,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct CacheConfig {
    pub provider_search: ProviderSearchCacheConfig,
    pub document: DocumentCacheConfig,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct DocumentCacheConfig {
    pub enabled: bool,
    pub ttl_seconds: u64,
    pub max_entries: u64,
    pub max_bytes: u64,
    pub store_content: bool,
    /// When non-zero, stale entries within this window are served while
    /// a background revalidation is triggered.
    pub stale_while_revalidate_seconds: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct DeepConfig {
    pub top_k: u32,
    pub max_fetches: u32,
    pub max_bytes: u64,
    pub max_redirects: u32,
    pub max_crawl_urls: u32,
    pub max_depth: u8,
    /// Consult `robots.txt` before fetching a link discovered by the crawl.
    /// URLs that came from Search are requested by the user and are not gated.
    pub respect_robots: bool,
    /// Deadline for one `robots.txt` retrieval.
    pub robots_timeout_ms: u64,
    /// Byte ceiling for one `robots.txt` retrieval.
    pub robots_max_bytes: u64,
    pub timeout_ms: u64,
    pub extractor: ExtractorConfig,
    pub renderer: RendererConfig,
    pub ranking_v2: RankingV2Config,
    pub gaps: GapConfig,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct RankingV2Config {
    pub enabled: bool,
    pub policy: RankingV2Policy,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct GapConfig {
    pub enabled: bool,
    pub max_subqueries: u32,
    pub max_cost: u64,
    pub max_provider_calls_per_subquery: u32,
    pub timeout_ms: u64,
    pub policy: GapPolicyV1,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ExtractorConfig {
    pub executable: String,
    pub version: String,
    pub timeout_ms: u64,
    pub max_output_bytes: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct RendererConfig {
    pub enabled: bool,
    pub max_browser_calls: u32,
    pub timeout_ms: u64,
    pub shutdown_grace_ms: u64,
    pub max_memory_mb: u64,
    pub max_redirects: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ProviderSearchCacheConfig {
    pub enabled: bool,
    pub ttl_seconds: u64,
    pub max_entries: u64,
    pub max_bytes: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct TelemetryConfig {
    pub persistence_enabled: bool,
    pub retention_days: u32,
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("unable to read configuration")]
    Read(#[from] std::io::Error),
    #[error("invalid configuration")]
    Parse(#[from] toml::de::Error),
    #[error("invalid result policy: {0}")]
    Policy(String),
}

impl ProviderRuntimeConfig {
    pub fn approved(&self) -> bool {
        let today = current_utc_day();
        self.approved_on(today)
    }

    fn approved_on(&self, today: i64) -> bool {
        let reviewed = self.reviewed_at.as_deref().and_then(day_from_iso_date);
        self.approval_status == ApprovalStatus::Approved
            && reviewed.is_some_and(|day| (day..=day + 90).contains(&today))
            && present(&self.adapter_version)
            && self
                .reviewer
                .as_deref()
                .is_some_and(|value| !value.is_empty())
            && present(&self.terms_version_or_date)
            && present(&self.allowed_access_method)
            && self
                .terms_url
                .as_deref()
                .is_some_and(|value| !value.is_empty())
            && self
                .plan_or_contract
                .as_deref()
                .is_some_and(|value| !value.is_empty())
            && present(&self.rate_limit)
            && present(&self.cost_model)
            && present(&self.data_handling_notes)
            && present(&self.operational_risk)
    }
}

fn present(value: &Option<String>) -> bool {
    value
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty())
}

fn current_utc_day() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| (duration.as_secs() / 86_400) as i64)
        .unwrap_or(i64::MAX)
}

fn day_from_iso_date(value: &str) -> Option<i64> {
    let mut parts = value.split('-');
    let year = parts.next()?.parse::<i64>().ok()?;
    let month = parts.next()?.parse::<i64>().ok()?;
    let day = parts.next()?.parse::<i64>().ok()?;
    if parts.next().is_some() || !(1..=12).contains(&month) {
        return None;
    }
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let max_day = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return None,
    };
    if !(1..=max_day).contains(&day) {
        return None;
    }
    let adjusted_year = year - i64::from(month <= 2);
    let era = if adjusted_year >= 0 {
        adjusted_year
    } else {
        adjusted_year - 399
    } / 400;
    let year_of_era = adjusted_year - era * 400;
    let shifted_month = month + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * shifted_month + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    Some(era * 146_097 + day_of_era - 719_468)
}

impl Default for ProviderRuntimeConfig {
    fn default() -> Self {
        Self {
            adapter_version: None,
            approval_status: ApprovalStatus::Draft,
            reviewed_at: None,
            reviewer: None,
            terms_url: None,
            terms_version_or_date: None,
            allowed_access_method: None,
            plan_or_contract: None,
            rate_limit: None,
            cost_model: None,
            credential_env: None,
            storage_rights: false,
            supported_regions: vec![],
            supported_filters: vec![],
            data_handling_notes: None,
            operational_risk: None,
        }
    }
}

impl Default for DataPolicyConfig {
    fn default() -> Self {
        Self {
            profile: SecurityProfile::Standard,
            egress: EgressPolicy::Governed,
            inference: InferenceMode::Disabled,
        }
    }
}

impl Default for ProviderConfig {
    fn default() -> Self {
        Self {
            enabled: vec![],
            declared: builtin_provider_records(),
        }
    }
}

/// Governance records shipped with AMATL, used as the base every configuration
/// file extends or overrides.
fn builtin_provider_records() -> std::collections::BTreeMap<String, ProviderRuntimeConfig> {
    let brave = ProviderRuntimeConfig {
        adapter_version: Some("brave-v1".into()),
        credential_env: Some("BRAVE_API_KEY".into()),
        terms_url: Some(
            "https://api-dashboard.search.brave.com/documentation/resources/terms-of-service"
                .into(),
        ),
        terms_version_or_date: Some("2026-02-11".into()),
        allowed_access_method: Some("official_api".into()),
        supported_filters: vec![
            "site".into(),
            "filetype".into(),
            "language".into(),
            "region".into(),
            "time_range".into(),
        ],
        ..ProviderRuntimeConfig::default()
    };
    let mojeek = ProviderRuntimeConfig {
        adapter_version: Some("mojeek-v1".into()),
        credential_env: Some("MOJEEK_API_KEY".into()),
        terms_url: Some("https://www.mojeek.com/support/api/".into()),
        allowed_access_method: Some("official_api".into()),
        ..ProviderRuntimeConfig::default()
    };
    std::collections::BTreeMap::from([
        ("brave".to_string(), brave),
        ("mojeek".to_string(), mojeek),
        (
            "duckduckgo_html".to_string(),
            ProviderRuntimeConfig::default(),
        ),
    ])
}

impl Default for TimeoutConfig {
    fn default() -> Self {
        Self {
            provider_ms: 3_000,
            global_ms: 8_000,
        }
    }
}

impl Default for BudgetConfig {
    fn default() -> Self {
        Self {
            max_provider_calls: 3,
        }
    }
}

impl Default for ExecutionConfig {
    fn default() -> Self {
        Self {
            global_concurrency: 4,
            per_provider_concurrency: 1,
            max_retries: 1,
            retry_jitter_ms: 25,
        }
    }
}

impl Default for PersistenceConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            path: "amatl.sqlite3".into(),
            history_enabled: true,
            saved_document_max_bytes: 1_048_576,
            audit_retention_days: crate::audit::AUDIT_DEFAULT_RETENTION_DAYS,
            history_retention_days: 90,
            cache_retention_days: 7,
            document_cache_retention_days: 30,
            purge_interval_seconds: 3600,
            auto_backup_enabled: false,
            auto_backup_interval_seconds: 86400,
            auto_backup_max_count: 7,
            backup_directory: None,
            locking_mode: SqliteLockingMode::Normal,
        }
    }
}

impl Default for ProviderSearchCacheConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            ttl_seconds: 300,
            max_entries: 10_000,
            max_bytes: 268_435_456,
        }
    }
}

impl Default for DocumentCacheConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            ttl_seconds: 86_400,
            max_entries: 1_000,
            max_bytes: 268_435_456,
            store_content: false,
            stale_while_revalidate_seconds: 0,
        }
    }
}

impl Default for DeepConfig {
    fn default() -> Self {
        Self {
            top_k: 5,
            max_fetches: 10,
            max_bytes: 20 * 1024 * 1024,
            max_redirects: 5,
            max_crawl_urls: 10,
            max_depth: 1,
            respect_robots: true,
            robots_timeout_ms: 3_000,
            robots_max_bytes: 512 * 1024,
            timeout_ms: 20_000,
            extractor: ExtractorConfig::default(),
            renderer: RendererConfig::default(),
            ranking_v2: RankingV2Config::default(),
            gaps: GapConfig::default(),
        }
    }
}

impl Default for RankingV2Config {
    fn default() -> Self {
        Self {
            enabled: true,
            policy: RankingV2Policy::default(),
        }
    }
}

impl Default for GapConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_subqueries: 2,
            max_cost: 2,
            max_provider_calls_per_subquery: 2,
            timeout_ms: 5_000,
            policy: GapPolicyV1::default(),
        }
    }
}

impl Default for ExtractorConfig {
    fn default() -> Self {
        Self {
            executable: "trafilatura".into(),
            version: "trafilatura-2.2.0-cli-json-v1".into(),
            timeout_ms: 8_000,
            max_output_bytes: 4 * 1024 * 1024,
        }
    }
}

impl Default for RendererConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            max_browser_calls: 2,
            timeout_ms: 8_000,
            shutdown_grace_ms: 500,
            max_memory_mb: 512,
            max_redirects: 5,
        }
    }
}

impl Default for TelemetryConfig {
    fn default() -> Self {
        Self {
            persistence_enabled: false,
            retention_days: 30,
        }
    }
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind: "127.0.0.1".into(),
            port: 8080,
            token_env: "AMATL_SERVER_TOKEN".into(),
            no_auth: false,
            clients: vec![],
            allowed_hosts: vec!["127.0.0.1".into(), "localhost".into(), "[::1]".into()],
            allowed_origins: vec![],
            max_body_bytes: 64 * 1024,
            max_header_bytes: 16 * 1024,
            request_timeout_ms: 30_000,
            idle_timeout_ms: 30_000,
            rate_limit_per_minute: 60,
            max_connections: 64,
            tls: TlsConfig::default(),
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            schema_version: crate::SCHEMA_VERSION.to_string(),
            data_policy: DataPolicyConfig::default(),
            inference: InferenceConfig::default(),
            providers: ProviderConfig::default(),
            timeouts: TimeoutConfig::default(),
            budget: BudgetConfig::default(),
            execution: ExecutionConfig::default(),
            ranking_policy: RankingPolicyV1::default(),
            diversity_policy: DiversityPolicyV1::default(),
            search_policy: SearchPolicyV1::default(),
            persistence: PersistenceConfig::default(),
            circuit_breaker: crate::circuit::CircuitPolicy::default(),
            cache: CacheConfig::default(),
            telemetry: TelemetryConfig::default(),
            deep: DeepConfig::default(),
            server: ServerConfig::default(),
        }
    }
}

impl Config {
    pub fn from_toml(input: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(input)
    }

    pub fn load_optional(path: &Path) -> Result<Self, ConfigError> {
        let config = match std::fs::read_to_string(path) {
            Ok(input) => Self::from_toml(&input)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Self::default(),
            Err(error) => return Err(error.into()),
        };
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.schema_version != crate::SCHEMA_VERSION {
            return Err(ConfigError::Policy(format!(
                "config schema_version '{}' does not match binary schema_version '{}'",
                self.schema_version,
                crate::SCHEMA_VERSION
            )));
        }
        if let Some(name) = self
            .providers
            .names()
            .into_iter()
            .find(|name| !is_provider_key(name))
        {
            return Err(ConfigError::Policy(format!(
                "invalid provider name: {name}; use lowercase ascii, digits and underscore"
            )));
        }
        if let Some(name) = self
            .providers
            .enabled
            .iter()
            .find(|name| !self.providers.is_declared(name))
        {
            return Err(ConfigError::Policy(format!(
                "unknown enabled provider: {name}"
            )));
        }
        self.validate_inference()?;
        if self.execution.global_concurrency == 0
            || self.execution.per_provider_concurrency == 0
            || self.execution.per_provider_concurrency > self.execution.global_concurrency
            || self.execution.max_retries > 2
            || self.execution.retry_jitter_ms > 1_000
        {
            return Err(ConfigError::Policy(
                "invalid parallel search execution limit".into(),
            ));
        }
        self.circuit_breaker
            .validate()
            .map_err(|error| ConfigError::Policy(error.into()))?;
        if !(1..=crate::audit::AUDIT_MAX_RETENTION_DAYS)
            .contains(&self.persistence.audit_retention_days)
        {
            return Err(ConfigError::Policy(
                "persistence.audit_retention_days must be between 1 and 365".into(),
            ));
        }
        if self.persistence.history_retention_days > 365 {
            return Err(ConfigError::Policy(
                "persistence.history_retention_days must be between 0 and 365".into(),
            ));
        }
        if self.persistence.cache_retention_days > 365 {
            return Err(ConfigError::Policy(
                "persistence.cache_retention_days must be between 0 and 365".into(),
            ));
        }
        if self.persistence.document_cache_retention_days > 365 {
            return Err(ConfigError::Policy(
                "persistence.document_cache_retention_days must be between 0 and 365".into(),
            ));
        }
        if self.persistence.purge_interval_seconds > 0
            && self.persistence.purge_interval_seconds < 60
        {
            return Err(ConfigError::Policy(
                "persistence.purge_interval_seconds must be 0 (disabled) or >= 60".into(),
            ));
        }
        if self.persistence.auto_backup_enabled {
            if self.persistence.auto_backup_interval_seconds < 3600 {
                return Err(ConfigError::Policy(
                    "persistence.auto_backup_interval_seconds must be >= 3600 (1 hour)".into(),
                ));
            }
            if self.persistence.auto_backup_max_count == 0
                || self.persistence.auto_backup_max_count > 365
            {
                return Err(ConfigError::Policy(
                    "persistence.auto_backup_max_count must be between 1 and 365".into(),
                ));
            }
            if let Some(directory) = self.persistence.backup_directory.as_deref() {
                if directory.trim().is_empty() {
                    return Err(ConfigError::Policy(
                        "persistence.backup_directory must not be empty".into(),
                    ));
                }
                if !std::path::Path::new(directory).is_absolute() {
                    return Err(ConfigError::Policy(
                        "persistence.backup_directory must be an absolute path".into(),
                    ));
                }
            }
        }
        if self.deep.respect_robots
            && (!(100..=30_000).contains(&self.deep.robots_timeout_ms)
                || !(1_024..=1_048_576).contains(&self.deep.robots_max_bytes))
        {
            return Err(ConfigError::Policy(
                "invalid robots.txt retrieval limit".into(),
            ));
        }
        if self.persistence.saved_document_max_bytes == 0
            || self.persistence.saved_document_max_bytes > 16 * 1024 * 1024
        {
            return Err(ConfigError::Policy(
                "persistence.saved_document_max_bytes must be between 1 and 16777216".into(),
            ));
        }
        self.ranking_policy
            .validate()
            .map_err(|error| ConfigError::Policy(error.to_string()))?;
        self.diversity_policy
            .validate()
            .map_err(|error| ConfigError::Policy(error.to_string()))?;
        self.search_policy
            .validate()
            .map_err(|error| ConfigError::Policy(error.to_string()))?;
        if self.search_policy.maximum_results_per_domain
            != self.diversity_policy.max_visible_per_domain
            || self.search_policy.maximum_results_per_provider
                != self.diversity_policy.max_visible_per_provider
            || self.search_policy.maximum_results_per_result_type
                != self.diversity_policy.max_visible_per_result_type
        {
            return Err(ConfigError::Policy(
                "search policy and diversity policy limits must agree".into(),
            ));
        }
        if self.cache.provider_search.ttl_seconds == 0
            || self.cache.provider_search.max_entries == 0
            || self.cache.provider_search.max_bytes == 0
            || self.cache.document.ttl_seconds == 0
            || self.cache.document.max_entries == 0
            || self.cache.document.max_bytes == 0
        {
            return Err(ConfigError::Policy("cache limits must be positive".into()));
        }
        if self.telemetry.retention_days < crate::telemetry::TELEMETRY_MIN_RETENTION_DAYS
            || self.telemetry.retention_days > crate::telemetry::TELEMETRY_MAX_RETENTION_DAYS
        {
            return Err(ConfigError::Policy(format!(
                "telemetry retention_days must be between {} and {}",
                crate::telemetry::TELEMETRY_MIN_RETENTION_DAYS,
                crate::telemetry::TELEMETRY_MAX_RETENTION_DAYS
            )));
        }
        if (self.cache.provider_search.enabled || self.telemetry.persistence_enabled)
            && !self.persistence.enabled
        {
            return Err(ConfigError::Policy(
                "SQLite persistence must be enabled for persistent cache or telemetry".into(),
            ));
        }
        if self.cache.document.enabled && !self.persistence.enabled {
            return Err(ConfigError::Policy(
                "SQLite persistence must be enabled for document cache".into(),
            ));
        }
        if self.deep.top_k == 0
            || self.deep.max_fetches == 0
            || self.deep.max_bytes == 0
            || self.deep.max_crawl_urls == 0
            || self.deep.max_depth > 2
            || self.deep.timeout_ms == 0
            || self.deep.extractor.timeout_ms == 0
            || self.deep.extractor.max_output_bytes == 0
            || self.deep.renderer.max_browser_calls == 0
            || self.deep.renderer.timeout_ms == 0
            || self.deep.renderer.shutdown_grace_ms == 0
            || self.deep.renderer.max_memory_mb == 0
        {
            return Err(ConfigError::Policy("invalid Deep safety limit".into()));
        }
        self.deep
            .ranking_v2
            .policy
            .validate()
            .map_err(|error| ConfigError::Policy(error.to_string()))?;
        self.deep
            .gaps
            .policy
            .validate()
            .map_err(|error| ConfigError::Policy(error.to_string()))?;
        if self.deep.gaps.max_subqueries > 2
            || self.deep.gaps.max_cost == 0
            || self.deep.gaps.max_provider_calls_per_subquery == 0
            || self.deep.gaps.timeout_ms == 0
        {
            return Err(ConfigError::Policy(
                "invalid Gap Analyzer safety limit".into(),
            ));
        }
        let bind = self
            .server
            .bind
            .parse::<std::net::IpAddr>()
            .map_err(|_| ConfigError::Policy("server bind must be an IP address".into()))?;
        if self.data_policy.profile == SecurityProfile::Isolated {
            if self.data_policy.egress != EgressPolicy::Deny {
                return Err(ConfigError::Policy(
                    "isolated profile requires denied network egress".into(),
                ));
            }
            if self.data_policy.inference == InferenceMode::RemoteExplicit {
                return Err(ConfigError::Policy(
                    "isolated profile forbids remote inference".into(),
                ));
            }
            if !bind.is_loopback() {
                return Err(ConfigError::Policy(
                    "isolated profile requires a loopback server bind".into(),
                ));
            }
            if self.deep.renderer.enabled {
                return Err(ConfigError::Policy(
                    "isolated profile forbids the unsandboxed renderer".into(),
                ));
            }
        }
        if self.data_policy.egress == EgressPolicy::Deny && !self.providers.enabled.is_empty() {
            return Err(ConfigError::Policy(
                "providers cannot be enabled while network egress is denied".into(),
            ));
        }
        if self.data_policy.egress == EgressPolicy::Deny
            && self.data_policy.inference == InferenceMode::RemoteExplicit
        {
            return Err(ConfigError::Policy(
                "remote inference requires governed network egress".into(),
            ));
        }
        let tls_complete =
            self.server.tls.cert_path.is_some() && self.server.tls.key_path.is_some();
        let tls_partial = self.server.tls.cert_path.is_some() != self.server.tls.key_path.is_some();
        if self.server.port == 0
            || self.server.token_env.trim().is_empty()
            || self.server.allowed_hosts.is_empty()
            || self.server.max_body_bytes == 0
            || self.server.max_body_bytes > 1024 * 1024
            || self.server.max_header_bytes == 0
            || self.server.max_header_bytes > 64 * 1024
            || self.server.request_timeout_ms == 0
            || self.server.idle_timeout_ms == 0
            || self.server.rate_limit_per_minute == 0
            || self.server.max_connections == 0
            || self.server.max_connections > 10_000
            || tls_partial
        {
            return Err(ConfigError::Policy(
                "invalid HTTP server safety limit".into(),
            ));
        }
        self.validate_clients()?;
        if !bind.is_loopback() && (self.server.no_auth || !tls_complete) {
            return Err(ConfigError::Policy(
                "remote HTTP bind requires authentication and TLS".into(),
            ));
        }
        if self.server.no_auth && !bind.is_loopback() {
            return Err(ConfigError::Policy(
                "no-auth development mode is loopback-only".into(),
            ));
        }
        if self.server.allowed_hosts.iter().any(|host| {
            host.trim().is_empty()
                || host.contains('*')
                || host.parse::<http::uri::Authority>().is_err()
        }) {
            return Err(ConfigError::Policy(
                "server allowed_hosts must contain exact HTTP authorities".into(),
            ));
        }
        if self.server.allowed_origins.iter().any(|origin| {
            url::Url::parse(origin).map_or(true, |url| {
                !matches!(url.scheme(), "http" | "https")
                    || url.host_str().is_none()
                    || url.path() != "/"
                    || url.query().is_some()
                    || url.fragment().is_some()
            })
        }) {
            return Err(ConfigError::Policy(
                "server allowed_origins must contain exact HTTP origins".into(),
            ));
        }
        Ok(())
    }

    /// Inference limits, and the policy that must back the ranking weights.
    ///
    /// Semantic and reranker weights are only accepted when an inference mode
    /// with an available backend is selected, so an operator never believes a
    /// semantic ranking is running while it silently falls back to lexical.
    /// Named credentials must be unambiguous, secret-free and capability-bound.
    fn validate_clients(&self) -> Result<(), ConfigError> {
        let mut seen = std::collections::BTreeSet::new();
        for client in &self.server.clients {
            if client.id.trim().is_empty() || !is_provider_key(&client.id) {
                return Err(ConfigError::Policy(format!(
                    "invalid server client id: {}; use lowercase ascii, digits and underscore",
                    client.id
                )));
            }
            if !seen.insert(client.id.as_str()) {
                return Err(ConfigError::Policy(format!(
                    "duplicate server client id: {}",
                    client.id
                )));
            }
            match (&client.token_env, &client.token_sha256) {
                (Some(name), None) if !name.trim().is_empty() => {}
                (None, Some(digest)) if is_sha256_hex(digest) => {}
                (None, Some(_)) => {
                    return Err(ConfigError::Policy(format!(
                        "client {} token_sha256 must be a lowercase hex SHA-256 digest",
                        client.id
                    )))
                }
                _ => {
                    return Err(ConfigError::Policy(format!(
                        "client {} must declare exactly one of token_env or token_sha256",
                        client.id
                    )))
                }
            }
            if client.scopes.is_empty() {
                return Err(ConfigError::Policy(format!(
                    "client {} declares no scope; a credential without capability is never useful",
                    client.id
                )));
            }
            if let Some(expiry) = client.expires_at.as_deref() {
                if day_from_iso_date(expiry).is_none() {
                    return Err(ConfigError::Policy(format!(
                        "client {} has an invalid expires_at; use YYYY-MM-DD",
                        client.id
                    )));
                }
            }
            if let Some(tool) = client
                .tools
                .iter()
                .find(|tool| !MCP_TOOLS.contains(&tool.as_str()))
            {
                return Err(ConfigError::Policy(format!(
                    "client {} allows unknown MCP tool: {tool}",
                    client.id
                )));
            }
            if !client.tools.is_empty() && !client.scopes.contains(&Scope::Mcp) {
                return Err(ConfigError::Policy(format!(
                    "client {} allows MCP tools without the mcp scope",
                    client.id
                )));
            }
        }
        Ok(())
    }

    fn validate_inference(&self) -> Result<(), ConfigError> {
        // `backend` selects the *local* embedder. The remote backend is chosen
        // by `data_policy.inference = "remote_explicit"`, never by this key, so
        // naming it here stays an error on purpose.
        match self.inference.backend.as_str() {
            crate::inference::LOCAL_EMBEDDING_BACKEND_ID => {}
            crate::inference::LOCAL_MODEL_BACKEND_ID => {
                let path = self
                    .inference
                    .local_model_path
                    .as_deref()
                    .filter(|value| !value.trim().is_empty())
                    .ok_or_else(|| {
                        ConfigError::Policy(format!(
                            "inference.local_model_path is required with backend = {}",
                            crate::inference::LOCAL_MODEL_BACKEND_ID
                        ))
                    })?;
                if !std::path::Path::new(path).is_absolute() {
                    return Err(ConfigError::Policy(
                        "inference.local_model_path must be an absolute path".into(),
                    ));
                }
            }
            other => {
                return Err(ConfigError::Policy(format!(
                    "unknown inference backend: {other} (expected {} or {}; the remote backend is selected by data_policy.inference)",
                    crate::inference::LOCAL_EMBEDDING_BACKEND_ID,
                    crate::inference::LOCAL_MODEL_BACKEND_ID
                )));
            }
        }
        if self.data_policy.inference == InferenceMode::RemoteExplicit {
            if !self.data_policy.allows_remote_inference() {
                return Err(ConfigError::Policy(
                    "remote_explicit inference requires a standard profile with governed egress"
                        .into(),
                ));
            }
            let endpoint = self.inference.remote_endpoint.as_deref().ok_or_else(|| {
                ConfigError::Policy(
                    "remote_explicit inference requires inference.remote_endpoint".into(),
                )
            })?;
            crate::inference::validate_remote_endpoint(endpoint)
                .map_err(|error| ConfigError::Policy(error.to_string()))?;
            if self
                .inference
                .remote_model
                .as_deref()
                .is_none_or(str::is_empty)
            {
                return Err(ConfigError::Policy(
                    "remote_explicit inference requires inference.remote_model".into(),
                ));
            }
            if !(1..=crate::inference::MAXIMUM_REMOTE_BATCH)
                .contains(&self.inference.remote_max_batch)
                || !(100..=60_000).contains(&self.inference.remote_timeout_ms)
            {
                return Err(ConfigError::Policy(
                    "invalid remote inference batch or timeout limit".into(),
                ));
            }
        }
        if !(crate::inference::MINIMUM_EMBEDDING_DIMENSIONS
            ..=crate::inference::MAXIMUM_EMBEDDING_DIMENSIONS)
            .contains(&self.inference.embedding_dimensions)
            || self.inference.max_documents == 0
            || self.inference.max_input_chars == 0
            || !self.inference.reranker_prior_weight.is_finite()
            || !(0.0..=1.0).contains(&self.inference.reranker_prior_weight)
        {
            return Err(ConfigError::Policy("invalid inference limit".into()));
        }
        let policy = &self.deep.ranking_v2.policy;
        let needs_backend = self.deep.ranking_v2.enabled
            && (policy.weight_semantic > 0.0 || policy.weight_reranker > 0.0);
        if needs_backend {
            match self.data_policy.inference {
                InferenceMode::Disabled => {
                    return Err(ConfigError::Policy(
                        "semantic or reranker ranking weights require an enabled inference mode"
                            .into(),
                    ))
                }
                // `remote_explicit` is validated above; reaching here means an
                // endpoint, model and bounded limits are all present.
                InferenceMode::RemoteExplicit | InferenceMode::LocalOnly => {}
            }
        }
        Ok(())
    }
}

/// Tool names the MCP surface exposes, and the vocabulary a client allowlist
/// may use. Kept here so an unknown name is rejected by configuration
/// validation instead of silently granting nothing.
pub const MCP_TOOLS: [&str; 5] = ["search", "deep_search", "fetch", "providers", "status"];

fn is_sha256_hex(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn is_provider_key(name: &str) -> bool {
    !name.is_empty()
        && name.chars().all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '_'
        })
}

fn default_schema_version() -> String {
    crate::SCHEMA_VERSION.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_safe_and_phase_one_shaped() {
        let config = Config::default();
        assert_eq!(config.data_policy.profile, SecurityProfile::Standard);
        assert_eq!(config.data_policy.egress, EgressPolicy::Governed);
        assert_eq!(config.data_policy.inference, InferenceMode::Disabled);
        assert!(config.data_policy.allows_network_egress());
        assert!(!config.data_policy.allows_local_inference());
        assert!(!config.data_policy.allows_remote_inference());
        assert!(config.providers.enabled.is_empty());
        assert!(!config.providers.get("brave").unwrap().approved());
        assert!(config.timeouts.provider_ms <= config.timeouts.global_ms);
        assert!(!config.deep.renderer.enabled);
        assert!(config.deep.max_depth <= 2);
        assert_eq!(config.server.bind, "127.0.0.1");
        assert!(!config.server.no_auth);
    }

    #[test]
    fn isolated_profile_is_fail_closed_and_allows_local_inference_only() {
        let mut config = Config::default();
        config.data_policy.profile = SecurityProfile::Isolated;
        config.data_policy.egress = EgressPolicy::Deny;
        config.data_policy.inference = InferenceMode::LocalOnly;
        assert!(config.validate().is_ok());
        assert!(!config.data_policy.allows_network_egress());
        assert!(config.data_policy.allows_local_inference());
        assert!(!config.data_policy.allows_remote_inference());
    }

    #[test]
    fn isolated_profile_rejects_contradictory_or_unsafe_configuration() {
        let mut governed = Config::default();
        governed.data_policy.profile = SecurityProfile::Isolated;
        assert!(governed.validate().is_err());
        assert!(!governed.data_policy.allows_network_egress());

        let mut remote = Config::default();
        remote.data_policy.profile = SecurityProfile::Isolated;
        remote.data_policy.egress = EgressPolicy::Deny;
        remote.data_policy.inference = InferenceMode::RemoteExplicit;
        assert!(remote.validate().is_err());
        assert!(!remote.data_policy.allows_remote_inference());

        let mut renderer = Config::default();
        renderer.data_policy.profile = SecurityProfile::Isolated;
        renderer.data_policy.egress = EgressPolicy::Deny;
        renderer.deep.renderer.enabled = true;
        assert!(renderer.validate().is_err());

        let mut remote_bind = Config::default();
        remote_bind.data_policy.profile = SecurityProfile::Isolated;
        remote_bind.data_policy.egress = EgressPolicy::Deny;
        remote_bind.server.bind = "0.0.0.0".into();
        remote_bind.server.tls.cert_path = Some("cert.pem".into());
        remote_bind.server.tls.key_path = Some("key.pem".into());
        assert!(remote_bind.validate().is_err());
    }

    #[test]
    fn denied_egress_rejects_network_providers_and_remote_inference() {
        let mut provider = Config::default();
        provider.data_policy.egress = EgressPolicy::Deny;
        provider.providers.enabled = vec!["brave".into()];
        assert!(provider.validate().is_err());

        let mut inference = Config::default();
        inference.data_policy.egress = EgressPolicy::Deny;
        inference.data_policy.inference = InferenceMode::RemoteExplicit;
        assert!(inference.validate().is_err());
        assert!(!inference.data_policy.allows_remote_inference());
    }

    #[test]
    fn rejects_unknown_provider_names_including_case_and_whitespace_typos() {
        for unknown in ["Brave", "brave ", "custom"] {
            let mut config = Config::default();
            config.providers.enabled = vec![unknown.into()];
            let error = config.validate().unwrap_err().to_string();
            assert!(error.contains("unknown enabled provider"), "{error}");
        }
    }

    #[test]
    fn approval_requires_a_complete_governance_record() {
        let incomplete = ProviderRuntimeConfig {
            approval_status: ApprovalStatus::Approved,
            ..ProviderRuntimeConfig::default()
        };
        assert!(!incomplete.approved());
        let complete = ProviderRuntimeConfig {
            adapter_version: Some("v1".into()),
            approval_status: ApprovalStatus::Approved,
            reviewed_at: Some("2026-08-12".into()),
            reviewer: Some("owner".into()),
            terms_url: Some("https://example.com/terms".into()),
            terms_version_or_date: Some("2026-08-01".into()),
            allowed_access_method: Some("official_api".into()),
            plan_or_contract: Some("contract-1".into()),
            rate_limit: Some("5 qps".into()),
            cost_model: Some("contract".into()),
            data_handling_notes: Some("no cache".into()),
            operational_risk: Some("quota".into()),
            ..ProviderRuntimeConfig::default()
        };
        assert!(complete.approved_on(day_from_iso_date("2026-08-12").unwrap()));
        assert!(!complete.approved_on(day_from_iso_date("2026-11-11").unwrap()));
    }

    #[test]
    fn expired_approval_is_never_active() {
        let config = ProviderRuntimeConfig {
            approval_status: ApprovalStatus::Expired,
            reviewed_at: Some("2025-01-01".into()),
            reviewer: Some("owner".into()),
            terms_url: Some("https://example.com/terms".into()),
            plan_or_contract: Some("contract-1".into()),
            ..ProviderRuntimeConfig::default()
        };
        assert!(!config.approved());
    }

    #[test]
    fn search_policy_is_configurable_but_rejected_when_invalid() {
        let mut config = Config::default();
        config.search_policy.minimum_expected_marginal_gain = 0.25;
        assert!(config.validate().is_ok());
        config.search_policy.first_round_min_providers = 4;
        config.search_policy.first_round_max_providers = 3;
        assert!(config.validate().is_err());
    }

    #[test]
    fn declared_providers_extend_the_builtin_records_without_losing_them() {
        let config = Config::from_toml(
            r#"
            [providers]
            enabled = ["custom_archive"]

            [providers.custom_archive]
            adapter_version = "custom-v1"
            credential_env = "CUSTOM_ARCHIVE_TOKEN"

            [providers.brave]
            adapter_version = "brave-pinned"
            "#,
        )
        .unwrap();
        assert!(config.validate().is_ok());
        assert_eq!(
            config.providers.names(),
            vec!["brave", "custom_archive", "duckduckgo_html", "mojeek"]
        );
        assert_eq!(
            config
                .providers
                .get("custom_archive")
                .unwrap()
                .credential_env
                .as_deref(),
            Some("CUSTOM_ARCHIVE_TOKEN")
        );
        assert_eq!(
            config.providers.get("brave").unwrap().adapter_version,
            Some("brave-pinned".into())
        );
        assert!(config.providers.is_declared("mojeek"));
        assert!(!config.providers.storage_rights("custom_archive"));
    }

    #[test]
    fn provider_names_must_be_stable_configuration_keys() {
        let mut config = Config::default();
        config
            .providers
            .declare("Brave Search", ProviderRuntimeConfig::default());
        let error = config.validate().unwrap_err().to_string();
        assert!(error.contains("invalid provider name"), "{error}");
    }

    #[test]
    fn semantic_ranking_weights_require_a_backed_inference_mode() {
        let mut config = Config::default();
        config.deep.ranking_v2.policy.weight_bm25 = 0.7;
        config.deep.ranking_v2.policy.weight_semantic = 0.3;
        let error = config.validate().unwrap_err().to_string();
        assert!(
            error.contains("require an enabled inference mode"),
            "{error}"
        );

        config.data_policy.inference = InferenceMode::LocalOnly;
        assert!(config.validate().is_ok());

        // remote_explicit is a real mode now, but it is fail-closed: it needs a
        // declared endpoint and model before any weight may rely on it.
        config.data_policy.inference = InferenceMode::RemoteExplicit;
        let error = config.validate().unwrap_err().to_string();
        assert!(
            error.contains("requires inference.remote_endpoint"),
            "{error}"
        );

        config.inference.remote_endpoint = Some("https://embeddings.invalid/v1".into());
        let error = config.validate().unwrap_err().to_string();
        assert!(error.contains("requires inference.remote_model"), "{error}");

        config.inference.remote_model = Some("text-embeddings".into());
        assert!(config.validate().is_ok());
    }

    #[test]
    fn remote_inference_endpoint_and_profile_are_governed() {
        let mut config = Config::default();
        config.data_policy.inference = InferenceMode::RemoteExplicit;
        config.inference.remote_model = Some("text-embeddings".into());

        // Plaintext off-host endpoints and embedded credentials are refused.
        config.inference.remote_endpoint = Some("http://embeddings.invalid/v1".into());
        assert!(config
            .validate()
            .unwrap_err()
            .to_string()
            .contains("must be https"));
        config.inference.remote_endpoint = Some("https://user:key@embeddings.invalid/v1".into());
        assert!(config
            .validate()
            .unwrap_err()
            .to_string()
            .contains("must not embed credentials"));

        // A self-hosted loopback endpoint is allowed over plain HTTP.
        config.inference.remote_endpoint = Some("http://127.0.0.1:11434/v1/embeddings".into());
        assert!(config.validate().is_ok());

        // An isolated profile can never reach a remote backend.
        config.data_policy.profile = SecurityProfile::Isolated;
        config.data_policy.egress = EgressPolicy::Deny;
        assert!(config.validate().is_err());

        // Batch and timeout bounds are enforced.
        let mut bounded = Config::default();
        bounded.data_policy.inference = InferenceMode::RemoteExplicit;
        bounded.inference.remote_endpoint = Some("https://embeddings.invalid/v1".into());
        bounded.inference.remote_model = Some("text-embeddings".into());
        bounded.inference.remote_max_batch = 0;
        assert!(bounded.validate().is_err());
        bounded.inference.remote_max_batch = 32;
        bounded.inference.remote_timeout_ms = 1;
        assert!(bounded.validate().is_err());
    }

    #[test]
    fn inference_limits_are_bounded() {
        let mut config = Config::default();
        config.inference.embedding_dimensions = 8;
        assert!(config.validate().is_err());

        let mut unknown = Config::default();
        unknown.inference.backend = "hosted".into();
        let error = unknown.validate().unwrap_err().to_string();
        assert!(error.contains("unknown inference backend"), "{error}");
    }

    #[test]
    fn local_model_backend_is_reachable_from_a_config_file() {
        // Regression: validation only accepted `local_hashing_v1`, so a TOML
        // selecting the documented `local_model_v1` failed to load and the
        // backend was unreachable end to end. Constructing `InferenceConfig`
        // directly (as the inference tests do) never exercised this path.
        let accepted = Config::from_toml(
            r#"
            [inference]
            backend = "local_model_v1"
            local_model_path = "/opt/amatl/vectors.txt"
            "#,
        )
        .unwrap();
        assert!(accepted.validate().is_ok(), "{:?}", accepted.validate());

        let missing_path = Config::from_toml(
            r#"
            [inference]
            backend = "local_model_v1"
            "#,
        )
        .unwrap();
        let error = missing_path.validate().unwrap_err().to_string();
        assert!(error.contains("local_model_path"), "{error}");

        let relative = Config::from_toml(
            r#"
            [inference]
            backend = "local_model_v1"
            local_model_path = "vectors.txt"
            "#,
        )
        .unwrap();
        let error = relative.validate().unwrap_err().to_string();
        assert!(error.contains("absolute"), "{error}");
    }

    #[test]
    fn remote_server_requires_tls_and_authentication() {
        let mut config = Config::default();
        config.server.bind = "0.0.0.0".into();
        assert!(config.validate().is_err());
        config.server.tls.cert_path = Some("cert.pem".into());
        config.server.tls.key_path = Some("key.pem".into());
        assert!(config.validate().is_ok());
        config.server.no_auth = true;
        assert!(config.validate().is_err());
    }

    #[test]
    fn schema_version_defaults_to_current_and_rejects_mismatch() {
        let config = Config::default();
        assert_eq!(config.schema_version, crate::SCHEMA_VERSION);
        assert!(config.validate().is_ok());

        let mismatched = Config {
            schema_version: "0".into(),
            ..Default::default()
        };
        let error = mismatched.validate().unwrap_err().to_string();
        assert!(
            error.contains("schema_version"),
            "expected schema_version error, got: {error}"
        );

        let future = Config {
            schema_version: "999".into(),
            ..Default::default()
        };
        assert!(future.validate().is_err());
    }

    #[test]
    fn toml_without_schema_version_defaults_to_current() {
        let config = Config::from_toml("").unwrap();
        assert_eq!(config.schema_version, crate::SCHEMA_VERSION);
        assert!(config.validate().is_ok());
    }
}
