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
    pub answer: AnswerConfig,
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

/// The [`DataPolicyConfig`] fields an admin-scoped caller may change in one
/// request, each optional so only the fields actually sent are changed.
///
/// Unlike every other config section, `data_policy`'s three fields cross-check
/// each other in `Config::validate` (`isolated` requires `deny`, `remote
/// explicit` requires `governed`), so this patch is applied and written as a
/// *unit* — one TOML read and one write — never as three independent writes
/// that could leave the file in a mix the next reload would reject. See
/// [`Config::set_data_policy_fields`].
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct DataPolicyConfigPatch {
    pub profile: Option<SecurityProfile>,
    pub egress: Option<EgressPolicy>,
    pub inference: Option<InferenceMode>,
}

impl DataPolicyConfigPatch {
    /// Apply every field this patch sets to `config`, leaving the rest
    /// untouched. Mirrors [`Config::set_data_policy_fields`] exactly.
    pub fn apply(&self, config: &mut DataPolicyConfig) {
        if let Some(value) = self.profile {
            config.profile = value;
        }
        if let Some(value) = self.egress {
            config.egress = value;
        }
        if let Some(value) = self.inference {
            config.inference = value;
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

/// Every [`InferenceConfig`] field an admin-scoped caller may update in one
/// request, each optional so only the fields actually sent are changed.
///
/// On the `Option<String>` fields that are themselves optional in
/// [`InferenceConfig`] (`local_model_path`, `local_cache_path`,
/// `remote_endpoint`, `remote_model`, `remote_credential_env`): submitting an
/// empty string clears the field (sets it back to `None`); omitting the
/// field from the request leaves it exactly as it was. There is no separate
/// "clear" flag — an admin who wants to blank a value out sends `""`.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct InferenceConfigPatch {
    pub backend: Option<String>,
    pub embedding_dimensions: Option<usize>,
    pub max_documents: Option<usize>,
    pub max_input_chars: Option<usize>,
    pub reranker_prior_weight: Option<f64>,
    pub local_model_path: Option<String>,
    pub local_model_batch: Option<usize>,
    pub local_cache_path: Option<String>,
    pub remote_endpoint: Option<String>,
    pub remote_model: Option<String>,
    pub remote_credential_env: Option<String>,
    pub remote_timeout_ms: Option<u64>,
    pub remote_max_batch: Option<usize>,
}

impl InferenceConfigPatch {
    /// Apply every field this patch sets to `config`, leaving the rest
    /// untouched.
    ///
    /// Mirrors exactly what [`Config::set_inference_fields`] writes to disk
    /// (same "empty string clears" convention on the optional string
    /// fields), so a caller that builds a candidate with this before
    /// validating and writing through that function never validates
    /// something different from what ends up on disk.
    pub fn apply(&self, config: &mut InferenceConfig) {
        if let Some(backend) = &self.backend {
            config.backend = backend.clone();
        }
        if let Some(value) = self.embedding_dimensions {
            config.embedding_dimensions = value;
        }
        if let Some(value) = self.max_documents {
            config.max_documents = value;
        }
        if let Some(value) = self.max_input_chars {
            config.max_input_chars = value;
        }
        if let Some(value) = self.reranker_prior_weight {
            config.reranker_prior_weight = value;
        }
        if let Some(value) = &self.local_model_path {
            config.local_model_path = clearable(value);
        }
        if let Some(value) = self.local_model_batch {
            config.local_model_batch = value;
        }
        if let Some(value) = &self.local_cache_path {
            config.local_cache_path = clearable(value);
        }
        if let Some(value) = &self.remote_endpoint {
            config.remote_endpoint = clearable(value);
        }
        if let Some(value) = &self.remote_model {
            config.remote_model = clearable(value);
        }
        if let Some(value) = &self.remote_credential_env {
            config.remote_credential_env = clearable(value);
        }
        if let Some(value) = self.remote_timeout_ms {
            config.remote_timeout_ms = value;
        }
        if let Some(value) = self.remote_max_batch {
            config.remote_max_batch = value;
        }
    }
}

/// `""` reads as "clear this field"; anything else is the new value.
/// Shared by every config patch's `apply` and its disk-writing counterpart
/// so the two never drift apart on what an empty string means.
fn clearable(value: &str) -> Option<String> {
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

/// Convert an unsigned patch integer to the `i64` `toml_edit` needs, failing
/// closed instead of wrapping.
///
/// `Config::validate` bounds every numeric field below `i64::MAX` before a
/// caller is allowed to write, so a value that reaches here is already in
/// range; this is the last line of defense for a future caller that forgets
/// to validate. Without it, `usize::MAX as i64` wraps to `-1` and lands in
/// the file, which the next `/reload` or restart refuses to parse.
fn signed_integer<T>(value: T, key: &str) -> Result<i64, ConfigError>
where
    T: TryInto<i64>,
{
    value
        .try_into()
        .map_err(|_| ConfigError::Policy(format!("{key} exceeds the maximum storable value")))
}

/// Governs the optional answer-synthesis step: an LLM completion call that
/// turns AMATL's own search results into a grounded, cited answer.
///
/// Disabled by default, and gated the same way as remote embeddings — it only
/// takes effect when `data_policy.inference = "remote_explicit"`, since it is
/// exactly that: a remote model call that leaves the machine. AMATL's search
/// and deep-fetch stay unaffected either way; this only adds a distinct,
/// explicitly invoked capability layered on top of results AMATL already
/// retrieved, never a replacement for them.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct AnswerConfig {
    pub enabled: bool,
    /// Chat-completions endpoint, OpenAI-compatible (`{"model":…,
    /// "messages":[…]}` → `choices[0].message.content`). Must be absolute
    /// HTTPS, or HTTP on loopback for a self-hosted server, and must not
    /// embed credentials — validated the same way as
    /// `inference.remote_endpoint`.
    pub endpoint: Option<String>,
    /// Model identifier sent in the completion request body.
    pub model: Option<String>,
    /// Environment variable holding the bearer credential. Never written to
    /// configuration or logs.
    pub credential_env: Option<String>,
    /// Deadline for one completion request. Bounded, not open-ended, so
    /// `answer` never turns into an unbounded wait on top of the search it
    /// already ran.
    pub timeout_ms: u64,
    /// Top-N ranked search results handed to the model as grounding sources.
    pub max_sources: usize,
    /// Characters of snippet kept per source before it reaches the prompt.
    pub max_source_chars: usize,
    /// Upper bound on the completion response length.
    pub max_answer_tokens: u32,
}

impl Default for AnswerConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            endpoint: None,
            model: None,
            credential_env: None,
            timeout_ms: 20_000,
            max_sources: 8,
            max_source_chars: 1_200,
            max_answer_tokens: 700,
        }
    }
}

/// Every [`AnswerConfig`] field an admin-scoped caller may update in one
/// request, except `enabled` — that stays the sole responsibility of
/// [`Config::set_answer_enabled`] (and the web UI's dedicated toggle), so
/// this patch can never accidentally flip the feature on or off as a side
/// effect of editing its endpoint or limits.
///
/// Same "empty string clears" convention on `endpoint`, `model` and
/// `credential_env` as [`InferenceConfigPatch`].
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct AnswerConfigPatch {
    pub endpoint: Option<String>,
    pub model: Option<String>,
    pub credential_env: Option<String>,
    pub timeout_ms: Option<u64>,
    pub max_sources: Option<usize>,
    pub max_source_chars: Option<usize>,
    pub max_answer_tokens: Option<u32>,
}

impl AnswerConfigPatch {
    /// Apply every field this patch sets to `config`, leaving `enabled` and
    /// every unset field untouched. Mirrors
    /// [`Config::set_answer_fields`] exactly; see
    /// [`InferenceConfigPatch::apply`] for the shared reasoning.
    pub fn apply(&self, config: &mut AnswerConfig) {
        if let Some(value) = &self.endpoint {
            config.endpoint = clearable(value);
        }
        if let Some(value) = &self.model {
            config.model = clearable(value);
        }
        if let Some(value) = &self.credential_env {
            config.credential_env = clearable(value);
        }
        if let Some(value) = self.timeout_ms {
            config.timeout_ms = value;
        }
        if let Some(value) = self.max_sources {
            config.max_sources = value;
        }
        if let Some(value) = self.max_source_chars {
            config.max_source_chars = value;
        }
        if let Some(value) = self.max_answer_tokens {
            config.max_answer_tokens = value;
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

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalStatus {
    #[default]
    Draft,
    Approved,
    Expired,
    Rejected,
}

impl ApprovalStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Approved => "approved",
            Self::Expired => "expired",
            Self::Rejected => "rejected",
        }
    }
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
    /// Redirect ceiling inherited by the fetch that produced the DOM.
    ///
    /// The renderer itself never navigates — it is handed bytes that
    /// [`crate::SafeFetcher`] already retrieved — so it contributes no
    /// redirects of its own.
    pub max_redirects: u32,
    /// Path to the `amatl-chromium-sandbox` isolation harness.
    ///
    /// Chromium is never launched directly: the harness is the only supported
    /// entry point because it is what enforces the network, memory, task and
    /// filesystem confinement the renderer's safety argument rests on.
    pub sandbox_path: String,
    /// Largest DOM the harness may return, in bytes.
    pub max_dom_bytes: u64,
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
///
/// Brave and Mojeek are declared `Rejected`, not `Draft`: both adapters are
/// implemented and tested, but both sources require a paid plan (Brave
/// dropped its free tier in 2026-02; Mojeek has none), and operator policy is
/// to run only no-cost sources. This is a deliberate, closed decision, not an
/// incomplete governance filing — `approved()` treats `Rejected` the same as
/// `Draft` (neither passes the gate), so the distinction changes nothing at
/// runtime; it exists so a future review (human or automated) reads the
/// status and finds a closed "no", not an invitation to fill in the paperwork
/// and enable a paid source. Re-approving either requires an explicit policy
/// change, not just completing the dossier. See «Viabilidad y coste» in
/// `docs/gobernanza-providers.md`.
fn builtin_provider_records() -> std::collections::BTreeMap<String, ProviderRuntimeConfig> {
    let brave = ProviderRuntimeConfig {
        adapter_version: Some("brave-v1".into()),
        approval_status: ApprovalStatus::Rejected,
        credential_env: Some("BRAVE_API_KEY".into()),
        terms_url: Some(
            "https://api-dashboard.search.brave.com/documentation/resources/terms-of-service"
                .into(),
        ),
        terms_version_or_date: Some("2026-02-11".into()),
        allowed_access_method: Some("official_api".into()),
        cost_model: Some(
            "Paid since 2026-02 (card required at signup; ~5 USD/mo credit, \
             usage billed beyond it)."
                .into(),
        ),
        operational_risk: Some(
            "Rejected by operator policy: no paid search providers. Do not \
             enable without an explicit policy change; see «Viabilidad y \
             coste» in docs/gobernanza-providers.md."
                .into(),
        ),
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
        approval_status: ApprovalStatus::Rejected,
        credential_env: Some("MOJEEK_API_KEY".into()),
        terms_url: Some("https://www.mojeek.com/support/api/".into()),
        allowed_access_method: Some("official_api".into()),
        cost_model: Some("Paid; no free tier.".into()),
        operational_risk: Some(
            "Rejected by operator policy: no paid search providers. Do not \
             enable without an explicit policy change; see «Viabilidad y \
             coste» in docs/gobernanza-providers.md."
                .into(),
        ),
        ..ProviderRuntimeConfig::default()
    };
    let searxng = ProviderRuntimeConfig {
        adapter_version: Some("searxng-v1".into()),
        credential_env: Some("SEARXNG_INSTANCE_URL".into()),
        terms_url: Some("https://docs.searxng.org/".into()),
        terms_version_or_date: Some("self-certified".into()),
        allowed_access_method: Some("self_hosted".into()),
        plan_or_contract: Some("self-hosted".into()),
        rate_limit: Some("unlimited (self-hosted)".into()),
        cost_model: Some("0".into()),
        storage_rights: false,
        data_handling_notes: Some(
            "Self-hosted SearXNG instance; no external terms. \
             Upstream engines have their own terms — operator must verify \
             compliance. No data stored by AMATL."
                .into(),
        ),
        operational_risk: Some(
            "Depends on upstream search engines that may block automated \
             access. IP reputation risk at volume. Operator should configure \
             only permissive engines."
                .into(),
        ),
        ..ProviderRuntimeConfig::default()
    };
    let marginalia = ProviderRuntimeConfig {
        adapter_version: Some("marginalia-v1".into()),
        credential_env: Some("MARGINALIA_API_KEY".into()),
        terms_url: Some("https://www.marginalia.nu/".into()),
        allowed_access_method: Some("official_api".into()),
        ..ProviderRuntimeConfig::default()
    };
    std::collections::BTreeMap::from([
        ("brave".to_string(), brave),
        ("mojeek".to_string(), mojeek),
        ("searxng".to_string(), searxng),
        ("marginalia".to_string(), marginalia),
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

/// The [`PersistenceConfig`] fields an admin-scoped caller may update in one
/// request: retention windows, the purge cadence and automatic-backup
/// settings.
///
/// Deliberately excludes `enabled`, `path` and `locking_mode` — those pick
/// *which* database file this process has open and how it coordinates with
/// other AMATL processes over it, which is not something to flip through a
/// running HTTP request; changing them stays a manual edit of the
/// configuration file followed by a restart.
///
/// Same "empty string clears" convention as [`InferenceConfigPatch`] on
/// `backup_directory` (clearing it falls back to the database's own
/// directory).
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct PersistenceConfigPatch {
    pub history_enabled: Option<bool>,
    pub saved_document_max_bytes: Option<u64>,
    pub audit_retention_days: Option<u32>,
    pub history_retention_days: Option<u32>,
    pub cache_retention_days: Option<u32>,
    pub document_cache_retention_days: Option<u32>,
    pub purge_interval_seconds: Option<u64>,
    pub auto_backup_enabled: Option<bool>,
    pub auto_backup_interval_seconds: Option<u64>,
    pub auto_backup_max_count: Option<u32>,
    pub backup_directory: Option<String>,
}

impl PersistenceConfigPatch {
    /// Apply every field this patch sets to `config`, leaving `enabled`,
    /// `path`, `locking_mode` and every unset field untouched. Mirrors
    /// [`Config::set_persistence_fields`] exactly.
    pub fn apply(&self, config: &mut PersistenceConfig) {
        if let Some(value) = self.history_enabled {
            config.history_enabled = value;
        }
        if let Some(value) = self.saved_document_max_bytes {
            config.saved_document_max_bytes = value;
        }
        if let Some(value) = self.audit_retention_days {
            config.audit_retention_days = value;
        }
        if let Some(value) = self.history_retention_days {
            config.history_retention_days = value;
        }
        if let Some(value) = self.cache_retention_days {
            config.cache_retention_days = value;
        }
        if let Some(value) = self.document_cache_retention_days {
            config.document_cache_retention_days = value;
        }
        if let Some(value) = self.purge_interval_seconds {
            config.purge_interval_seconds = value;
        }
        if let Some(value) = self.auto_backup_enabled {
            config.auto_backup_enabled = value;
        }
        if let Some(value) = self.auto_backup_interval_seconds {
            config.auto_backup_interval_seconds = value;
        }
        if let Some(value) = self.auto_backup_max_count {
            config.auto_backup_max_count = value;
        }
        if let Some(value) = &self.backup_directory {
            config.backup_directory = clearable(value);
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
            sandbox_path: "amatl-chromium-sandbox".into(),
            max_dom_bytes: 8 * 1024 * 1024,
        }
    }
}

/// The [`DeepConfig`] top-level safety limits an admin-scoped caller may
/// update in one request. Deliberately excludes `extractor`, `renderer`,
/// `ranking_v2` and `gaps` — each of those is its own nested table with its
/// own patch type ([`ExtractorConfigPatch`], [`RendererConfigPatch`]) or, for
/// `ranking_v2.policy`/`gaps.policy`, its own `/policies/*` endpoint (see
/// `Config::set_ranking_v2_policy`/`set_gap_policy`), rather than being
/// folded into this one.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct DeepConfigPatch {
    pub top_k: Option<u32>,
    pub max_fetches: Option<u32>,
    pub max_bytes: Option<u64>,
    pub max_redirects: Option<u32>,
    pub max_crawl_urls: Option<u32>,
    pub max_depth: Option<u8>,
    pub respect_robots: Option<bool>,
    pub robots_timeout_ms: Option<u64>,
    pub robots_max_bytes: Option<u64>,
    pub timeout_ms: Option<u64>,
}

impl DeepConfigPatch {
    /// Apply every field this patch sets to `config`, leaving `extractor`,
    /// `renderer`, `ranking_v2` and `gaps` untouched. Mirrors
    /// [`Config::set_deep_fields`] exactly.
    pub fn apply(&self, config: &mut DeepConfig) {
        if let Some(value) = self.top_k {
            config.top_k = value;
        }
        if let Some(value) = self.max_fetches {
            config.max_fetches = value;
        }
        if let Some(value) = self.max_bytes {
            config.max_bytes = value;
        }
        if let Some(value) = self.max_redirects {
            config.max_redirects = value;
        }
        if let Some(value) = self.max_crawl_urls {
            config.max_crawl_urls = value;
        }
        if let Some(value) = self.max_depth {
            config.max_depth = value;
        }
        if let Some(value) = self.respect_robots {
            config.respect_robots = value;
        }
        if let Some(value) = self.robots_timeout_ms {
            config.robots_timeout_ms = value;
        }
        if let Some(value) = self.robots_max_bytes {
            config.robots_max_bytes = value;
        }
        if let Some(value) = self.timeout_ms {
            config.timeout_ms = value;
        }
    }
}

/// Every [`ExtractorConfig`] field an admin-scoped caller may update.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct ExtractorConfigPatch {
    pub executable: Option<String>,
    pub version: Option<String>,
    pub timeout_ms: Option<u64>,
    pub max_output_bytes: Option<u64>,
}

impl ExtractorConfigPatch {
    /// Apply every field this patch sets to `config`. Mirrors
    /// [`Config::set_deep_extractor_fields`] exactly. Unlike the
    /// `Option<String>` fields on [`InferenceConfigPatch`], `executable` and
    /// `version` are themselves required (non-optional) strings in
    /// [`ExtractorConfig`], so there is no "clear" case to support here —
    /// `Some(value)` always sets a real replacement.
    pub fn apply(&self, config: &mut ExtractorConfig) {
        if let Some(value) = &self.executable {
            config.executable = value.clone();
        }
        if let Some(value) = &self.version {
            config.version = value.clone();
        }
        if let Some(value) = self.timeout_ms {
            config.timeout_ms = value;
        }
        if let Some(value) = self.max_output_bytes {
            config.max_output_bytes = value;
        }
    }
}

/// Every [`RendererConfig`] field an admin-scoped caller may update.
///
/// Setting `enabled = true` while `data_policy.profile = "isolated"` fails
/// `Config::validate` (an isolated profile forbids the unsandboxed
/// renderer), so a candidate built with this patch and validated before
/// writing already fails closed on that combination — see
/// [`Config::set_deep_renderer_fields`].
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct RendererConfigPatch {
    pub enabled: Option<bool>,
    pub max_browser_calls: Option<u32>,
    pub timeout_ms: Option<u64>,
    pub shutdown_grace_ms: Option<u64>,
    pub max_memory_mb: Option<u64>,
    pub max_redirects: Option<u32>,
    pub sandbox_path: Option<String>,
    pub max_dom_bytes: Option<u64>,
}

impl RendererConfigPatch {
    /// Apply every field this patch sets to `config`. Mirrors
    /// [`Config::set_deep_renderer_fields`] exactly.
    pub fn apply(&self, config: &mut RendererConfig) {
        if let Some(value) = self.enabled {
            config.enabled = value;
        }
        if let Some(value) = self.max_browser_calls {
            config.max_browser_calls = value;
        }
        if let Some(value) = self.timeout_ms {
            config.timeout_ms = value;
        }
        if let Some(value) = self.shutdown_grace_ms {
            config.shutdown_grace_ms = value;
        }
        if let Some(value) = self.max_memory_mb {
            config.max_memory_mb = value;
        }
        if let Some(value) = self.max_redirects {
            config.max_redirects = value;
        }
        if let Some(value) = &self.sandbox_path {
            config.sandbox_path = value.clone();
        }
        if let Some(value) = self.max_dom_bytes {
            config.max_dom_bytes = value;
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

/// Every [`TelemetryConfig`] field an admin-scoped caller may update.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct TelemetryConfigPatch {
    pub persistence_enabled: Option<bool>,
    pub retention_days: Option<u32>,
}

impl TelemetryConfigPatch {
    /// Apply every field this patch sets to `config`. Mirrors
    /// [`Config::set_telemetry_fields`] exactly.
    pub fn apply(&self, config: &mut TelemetryConfig) {
        if let Some(value) = self.persistence_enabled {
            config.persistence_enabled = value;
        }
        if let Some(value) = self.retention_days {
            config.retention_days = value;
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

/// Every [`ServerConfig`] field an admin-scoped caller may update in one
/// request, except `clients` — that has its own dedicated surface
/// ([`Config::upsert_server_client`]/[`Config::remove_server_client`], and
/// `/server/clients*` in `amatl-server`) because credentials need
/// create/rotate/revoke semantics no generic field patch should also try to
/// express.
///
/// Unlike every other patch type in this module, applying this one does
/// *not* mean every field takes effect once the file is written and the
/// process reloads: several of these are read once at process startup (see
/// [`ReloadKind`]) and only take full effect on restart. This patch does
/// not hide that — it is the caller's job (see `PATCH
/// /server/pending-config` in `amatl-server`) to report, per field it
/// wrote, whether [`ReloadKind::of`] says `Hot` or `Cold`.
///
/// Same "empty string clears" convention as [`InferenceConfigPatch`] on
/// `tls_cert_path`/`tls_key_path` (clearing either drops back to plaintext
/// HTTP — `Config::validate` still enforces that a remote bind requires
/// both or neither).
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct ServerConfigPatch {
    pub bind: Option<String>,
    pub port: Option<u16>,
    pub token_env: Option<String>,
    pub no_auth: Option<bool>,
    pub allowed_hosts: Option<Vec<String>>,
    pub allowed_origins: Option<Vec<String>>,
    pub max_body_bytes: Option<usize>,
    pub max_header_bytes: Option<usize>,
    pub request_timeout_ms: Option<u64>,
    pub idle_timeout_ms: Option<u64>,
    pub rate_limit_per_minute: Option<u32>,
    pub max_connections: Option<usize>,
    pub tls_cert_path: Option<String>,
    pub tls_key_path: Option<String>,
}

impl ServerConfigPatch {
    /// Apply every field this patch sets to `config`, leaving `clients` and
    /// every unset field untouched. Mirrors [`Config::set_server_fields`]
    /// exactly.
    pub fn apply(&self, config: &mut ServerConfig) {
        if let Some(value) = &self.bind {
            config.bind = value.clone();
        }
        if let Some(value) = self.port {
            config.port = value;
        }
        if let Some(value) = &self.token_env {
            config.token_env = value.clone();
        }
        if let Some(value) = self.no_auth {
            config.no_auth = value;
        }
        if let Some(value) = &self.allowed_hosts {
            config.allowed_hosts = value.clone();
        }
        if let Some(value) = &self.allowed_origins {
            config.allowed_origins = value.clone();
        }
        if let Some(value) = self.max_body_bytes {
            config.max_body_bytes = value;
        }
        if let Some(value) = self.max_header_bytes {
            config.max_header_bytes = value;
        }
        if let Some(value) = self.request_timeout_ms {
            config.request_timeout_ms = value;
        }
        if let Some(value) = self.idle_timeout_ms {
            config.idle_timeout_ms = value;
        }
        if let Some(value) = self.rate_limit_per_minute {
            config.rate_limit_per_minute = value;
        }
        if let Some(value) = self.max_connections {
            config.max_connections = value;
        }
        if let Some(value) = &self.tls_cert_path {
            config.tls.cert_path = clearable(value);
        }
        if let Some(value) = &self.tls_key_path {
            config.tls.key_path = clearable(value);
        }
    }

    /// `[section, key]` pairs for every field this patch actually sets, in
    /// the shape [`ReloadKind::of`] expects — `tls_cert_path`/`tls_key_path`
    /// report as `("server.tls", "cert_path"/"key_path")`, everything else
    /// as `("server", <field name>)`. Lets a caller classify exactly what it
    /// wrote without re-deriving the field list or the `tls_*` renaming.
    pub fn changed_fields(&self) -> Vec<(&'static str, &'static str)> {
        let mut fields = Vec::new();
        macro_rules! push_if_set {
            ($field:ident, $section:expr, $key:expr) => {
                if self.$field.is_some() {
                    fields.push(($section, $key));
                }
            };
        }
        push_if_set!(bind, "server", "bind");
        push_if_set!(port, "server", "port");
        push_if_set!(token_env, "server", "token_env");
        push_if_set!(no_auth, "server", "no_auth");
        push_if_set!(allowed_hosts, "server", "allowed_hosts");
        push_if_set!(allowed_origins, "server", "allowed_origins");
        push_if_set!(max_body_bytes, "server", "max_body_bytes");
        push_if_set!(max_header_bytes, "server", "max_header_bytes");
        push_if_set!(request_timeout_ms, "server", "request_timeout_ms");
        push_if_set!(idle_timeout_ms, "server", "idle_timeout_ms");
        push_if_set!(rate_limit_per_minute, "server", "rate_limit_per_minute");
        push_if_set!(max_connections, "server", "max_connections");
        push_if_set!(tls_cert_path, "server.tls", "cert_path");
        push_if_set!(tls_key_path, "server.tls", "key_path");
        fields
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            schema_version: crate::SCHEMA_VERSION.to_string(),
            data_policy: DataPolicyConfig::default(),
            inference: InferenceConfig::default(),
            answer: AnswerConfig::default(),
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

    /// Write `[section].key = value` to the file at `path`, creating the
    /// section table if it is absent, and nothing else.
    ///
    /// This is the write primitive every narrow, admin-scoped config
    /// mutation shares — today that is [`Config::set_answer_enabled`], and it
    /// exists precisely so a future toggle (another scalar field under
    /// `Config`) can reuse the same guarantee without re-deriving it: uses
    /// `toml_edit`, not a struct round-trip through `toml::to_string`,
    /// specifically so every comment an operator (or this project) wrote in
    /// the file survives the edit untouched.
    ///
    /// The caller is expected to have already checked that the resulting
    /// configuration will validate (build it in memory with the change
    /// applied and call `.validate()`) before calling this — this function
    /// only performs the write, so a file is never left in a state the
    /// running process didn't already agree to serve.
    fn set_scalar_field(
        path: &Path,
        section: &str,
        key: &str,
        value: impl Into<toml_edit::Value>,
    ) -> Result<(), ConfigError> {
        let text = std::fs::read_to_string(path)?;
        let mut document = text
            .parse::<toml_edit::DocumentMut>()
            .map_err(|error| ConfigError::Policy(format!("invalid TOML: {error}")))?;
        if document.get(section).is_none() {
            document[section] = toml_edit::Item::Table(toml_edit::Table::new());
        }
        document[section][key] = toml_edit::value(value);
        std::fs::write(path, document.to_string())?;
        Ok(())
    }

    /// Add or remove `item` from the string array at `[section].key`, and
    /// nothing else.
    ///
    /// The write primitive behind [`Config::set_provider_enabled`], kept
    /// generic so a future admin-scoped toggle over another string-array
    /// field does not need to re-derive the same `toml_edit` membership
    /// logic. Same guarantees as `Config::set_scalar_field`: comments
    /// survive, and the caller must validate a candidate config before
    /// calling this.
    fn set_list_membership(
        path: &Path,
        section: &str,
        key: &str,
        item: &str,
        present: bool,
    ) -> Result<(), ConfigError> {
        let text = std::fs::read_to_string(path)?;
        let mut document = text
            .parse::<toml_edit::DocumentMut>()
            .map_err(|error| ConfigError::Policy(format!("invalid TOML: {error}")))?;
        if document.get(section).is_none() {
            document[section] = toml_edit::Item::Table(toml_edit::Table::new());
        }
        let list = document[section]
            .as_table_mut()
            .ok_or_else(|| ConfigError::Policy(format!("{section} is not a table")))?
            .entry(key)
            .or_insert_with(|| toml_edit::Item::Value(toml_edit::Array::new().into()));
        let array = list
            .as_array_mut()
            .ok_or_else(|| ConfigError::Policy(format!("{section}.{key} is not an array")))?;
        let already_present = array.iter().any(|value| value.as_str() == Some(item));
        if present && !already_present {
            array.push(item);
        } else if !present && already_present {
            let index = array
                .iter()
                .position(|value| value.as_str() == Some(item))
                .expect("already_present implies a match");
            array.remove(index);
        }
        std::fs::write(path, document.to_string())?;
        Ok(())
    }

    /// Flip `[answer].enabled` on disk and nothing else.
    ///
    /// The only configuration mutation any running AMATL process makes to
    /// its own file — every other setting is still operator-edited text.
    /// Kept this narrow on purpose: it exists so the web UI's admin-scoped
    /// toggle never needs to touch, or even see, `endpoint`/`model`/the
    /// credential. A thin wrapper over `Config::set_scalar_field`; see
    /// that function for the write guarantees (comments survive, caller
    /// validates first).
    pub fn set_answer_enabled(path: &Path, enabled: bool) -> Result<(), ConfigError> {
        Self::set_scalar_field(path, "answer", "enabled", enabled)
    }

    /// Add or remove one name from `providers.enabled` and nothing else.
    ///
    /// A thin wrapper over `Config::set_list_membership`; see that
    /// function for the write guarantees (comments survive, caller
    /// validates first). This never touches a provider's approval ficha —
    /// governance still gates whether an enabled name actually sends
    /// traffic (`approved()` is checked again at call time), so flipping
    /// this switch on an unapproved or undeclared name degrades to
    /// `provider_not_approved` rather than bypassing the gate.
    pub fn set_provider_enabled(path: &Path, name: &str, enabled: bool) -> Result<(), ConfigError> {
        if !is_provider_key(name) {
            return Err(ConfigError::Policy(format!(
                "invalid provider name: {name}"
            )));
        }
        Self::set_list_membership(path, "providers", "enabled", name, enabled)
    }

    /// Insert a new `[[server.clients]]` entry, or replace the one with the
    /// same `id`, and nothing else.
    ///
    /// Same guarantees as `Config::set_scalar_field`: `toml_edit`, so every
    /// comment survives; the caller is expected to have already validated a
    /// candidate config with this entry applied (in particular, that `id` is
    /// a valid, unique client key — see `Config::validate_clients`). The
    /// credential itself is never handled here: `client.token_sha256` (or
    /// `token_env`) is written exactly as given, so a caller minting a new
    /// client must hash the raw token *before* calling this and hand the raw
    /// value to the operator exactly once — it is never reconstructible from
    /// what gets written to disk.
    pub fn upsert_server_client(path: &Path, client: &ServerClient) -> Result<(), ConfigError> {
        let text = std::fs::read_to_string(path)?;
        let mut document = text
            .parse::<toml_edit::DocumentMut>()
            .map_err(|error| ConfigError::Policy(format!("invalid TOML: {error}")))?;
        if document.get("server").is_none() {
            document["server"] = toml_edit::Item::Table(toml_edit::Table::new());
        }
        let server_table = document["server"]
            .as_table_mut()
            .ok_or_else(|| ConfigError::Policy("server is not a table".into()))?;
        if server_table.get("clients").is_none() {
            server_table.insert(
                "clients",
                toml_edit::Item::ArrayOfTables(toml_edit::ArrayOfTables::new()),
            );
        }
        let clients = server_table["clients"]
            .as_array_of_tables_mut()
            .ok_or_else(|| {
                ConfigError::Policy("server.clients is not an array of tables".into())
            })?;
        // Replacing in place at the discovered index keeps a stable read
        // order for an operator diffing the file; client order carries no
        // runtime meaning either way, since `id` is the only identity
        // `validate_clients` enforces unique.
        let table = server_client_to_table(client);
        let existing_index = clients.iter().position(|table| {
            table.get("id").and_then(|item| item.as_str()) == Some(client.id.as_str())
        });
        match existing_index.and_then(|index| clients.get_mut(index)) {
            Some(existing) => *existing = table,
            None => clients.push(table),
        }
        std::fs::write(path, document.to_string())?;
        Ok(())
    }

    /// Remove one `[[server.clients]]` entry by id, and nothing else.
    ///
    /// A no-op, not an error, when `id` is not declared — the caller already
    /// knows whether it expected a removal from the config it read before
    /// calling this.
    pub fn remove_server_client(path: &Path, id: &str) -> Result<(), ConfigError> {
        let text = std::fs::read_to_string(path)?;
        let mut document = text
            .parse::<toml_edit::DocumentMut>()
            .map_err(|error| ConfigError::Policy(format!("invalid TOML: {error}")))?;
        if let Some(clients) = document
            .get_mut("server")
            .and_then(|server| server.get_mut("clients"))
            .and_then(|item| item.as_array_of_tables_mut())
        {
            clients.retain(|table| table.get("id").and_then(|item| item.as_str()) != Some(id));
        }
        std::fs::write(path, document.to_string())?;
        Ok(())
    }

    /// Write `[data_policy].profile` and nothing else.
    ///
    /// A thin wrapper over `Config::set_scalar_field`, same as
    /// [`Config::set_answer_enabled`]: the caller validates a candidate
    /// config first, this only performs the write.
    pub fn set_data_policy_profile(
        path: &Path,
        profile: SecurityProfile,
    ) -> Result<(), ConfigError> {
        Self::set_scalar_field(path, "data_policy", "profile", profile.as_str())
    }

    /// Write `[data_policy].egress` and nothing else. See
    /// [`Config::set_data_policy_profile`].
    pub fn set_data_policy_egress(path: &Path, egress: EgressPolicy) -> Result<(), ConfigError> {
        Self::set_scalar_field(path, "data_policy", "egress", egress.as_str())
    }

    /// Write `[data_policy].inference` and nothing else. See
    /// [`Config::set_data_policy_profile`].
    pub fn set_data_policy_inference(
        path: &Path,
        inference: InferenceMode,
    ) -> Result<(), ConfigError> {
        Self::set_scalar_field(path, "data_policy", "inference", inference.as_str())
    }

    /// Write every field `patch` sets to `[data_policy]` — and nothing else —
    /// in a single read and a single write.
    ///
    /// This is deliberately the *whole-section* setter for `data_policy`, not
    /// three calls to the per-field setters above: the three fields cross-check
    /// each other in `Config::validate` (`isolated` requires `deny`, `remote
    /// explicit` requires `governed`), and three independent writes could
    /// leave the file in a mix `validate()` would reject if the second one
    /// failed — a file the next `/reload` or restart would then refuse. One
    /// `std::fs::write` is atomic at the filesystem level, so the file is
    /// either the old state or the fully applied new state. Same guarantees as
    /// `Config::set_scalar_field`: comments survive, and the caller must
    /// validate a candidate config first.
    pub fn set_data_policy_fields(
        path: &Path,
        patch: &DataPolicyConfigPatch,
    ) -> Result<(), ConfigError> {
        let text = std::fs::read_to_string(path)?;
        let mut document = text
            .parse::<toml_edit::DocumentMut>()
            .map_err(|error| ConfigError::Policy(format!("invalid TOML: {error}")))?;
        if document.get("data_policy").is_none() {
            document["data_policy"] = toml_edit::Item::Table(toml_edit::Table::new());
        }
        let table = document["data_policy"]
            .as_table_mut()
            .ok_or_else(|| ConfigError::Policy("data_policy is not a table".into()))?;
        if let Some(profile) = patch.profile {
            table.insert("profile", toml_edit::value(profile.as_str()));
        }
        if let Some(egress) = patch.egress {
            table.insert("egress", toml_edit::value(egress.as_str()));
        }
        if let Some(inference) = patch.inference {
            table.insert("inference", toml_edit::value(inference.as_str()));
        }
        std::fs::write(path, document.to_string())?;
        Ok(())
    }

    /// Write every field `patch` sets to `[inference]`, and nothing else.
    ///
    /// Same guarantees as `Config::set_scalar_field` (comments survive,
    /// caller validates a candidate first) — see
    /// [`InferenceConfigPatch::apply`], which every field here mirrors.
    pub fn set_inference_fields(
        path: &Path,
        patch: &InferenceConfigPatch,
    ) -> Result<(), ConfigError> {
        let text = std::fs::read_to_string(path)?;
        let mut document = text
            .parse::<toml_edit::DocumentMut>()
            .map_err(|error| ConfigError::Policy(format!("invalid TOML: {error}")))?;
        if document.get("inference").is_none() {
            document["inference"] = toml_edit::Item::Table(toml_edit::Table::new());
        }
        let table = document["inference"]
            .as_table_mut()
            .ok_or_else(|| ConfigError::Policy("inference is not a table".into()))?;
        if let Some(backend) = &patch.backend {
            table.insert("backend", toml_edit::value(backend.as_str()));
        }
        if let Some(value) = patch.embedding_dimensions {
            table.insert(
                "embedding_dimensions",
                toml_edit::value(signed_integer(value, "inference.embedding_dimensions")?),
            );
        }
        if let Some(value) = patch.max_documents {
            table.insert(
                "max_documents",
                toml_edit::value(signed_integer(value, "inference.max_documents")?),
            );
        }
        if let Some(value) = patch.max_input_chars {
            table.insert(
                "max_input_chars",
                toml_edit::value(signed_integer(value, "inference.max_input_chars")?),
            );
        }
        if let Some(value) = patch.reranker_prior_weight {
            table.insert("reranker_prior_weight", toml_edit::value(value));
        }
        set_clearable_string(table, "local_model_path", patch.local_model_path.as_deref());
        if let Some(value) = patch.local_model_batch {
            table.insert(
                "local_model_batch",
                toml_edit::value(signed_integer(value, "inference.local_model_batch")?),
            );
        }
        set_clearable_string(table, "local_cache_path", patch.local_cache_path.as_deref());
        set_clearable_string(table, "remote_endpoint", patch.remote_endpoint.as_deref());
        set_clearable_string(table, "remote_model", patch.remote_model.as_deref());
        set_clearable_string(
            table,
            "remote_credential_env",
            patch.remote_credential_env.as_deref(),
        );
        if let Some(value) = patch.remote_timeout_ms {
            table.insert(
                "remote_timeout_ms",
                toml_edit::value(signed_integer(value, "inference.remote_timeout_ms")?),
            );
        }
        if let Some(value) = patch.remote_max_batch {
            table.insert(
                "remote_max_batch",
                toml_edit::value(signed_integer(value, "inference.remote_max_batch")?),
            );
        }
        std::fs::write(path, document.to_string())?;
        Ok(())
    }

    /// Write every field `patch` sets to `[answer]`, and nothing else —
    /// `enabled` is never touched here (see [`AnswerConfigPatch`]). Same
    /// guarantees as [`Config::set_inference_fields`].
    pub fn set_answer_fields(path: &Path, patch: &AnswerConfigPatch) -> Result<(), ConfigError> {
        let text = std::fs::read_to_string(path)?;
        let mut document = text
            .parse::<toml_edit::DocumentMut>()
            .map_err(|error| ConfigError::Policy(format!("invalid TOML: {error}")))?;
        if document.get("answer").is_none() {
            document["answer"] = toml_edit::Item::Table(toml_edit::Table::new());
        }
        let table = document["answer"]
            .as_table_mut()
            .ok_or_else(|| ConfigError::Policy("answer is not a table".into()))?;
        set_clearable_string(table, "endpoint", patch.endpoint.as_deref());
        set_clearable_string(table, "model", patch.model.as_deref());
        set_clearable_string(table, "credential_env", patch.credential_env.as_deref());
        if let Some(value) = patch.timeout_ms {
            table.insert(
                "timeout_ms",
                toml_edit::value(signed_integer(value, "answer.timeout_ms")?),
            );
        }
        if let Some(value) = patch.max_sources {
            table.insert(
                "max_sources",
                toml_edit::value(signed_integer(value, "answer.max_sources")?),
            );
        }
        if let Some(value) = patch.max_source_chars {
            table.insert(
                "max_source_chars",
                toml_edit::value(signed_integer(value, "answer.max_source_chars")?),
            );
        }
        if let Some(value) = patch.max_answer_tokens {
            table.insert(
                "max_answer_tokens",
                toml_edit::value(signed_integer(value, "answer.max_answer_tokens")?),
            );
        }
        std::fs::write(path, document.to_string())?;
        Ok(())
    }

    /// Write every field `patch` sets to `[persistence]`, and nothing else —
    /// `enabled`, `path` and `locking_mode` are never touched here (see
    /// [`PersistenceConfigPatch`]). Same guarantees as
    /// [`Config::set_inference_fields`].
    pub fn set_persistence_fields(
        path: &Path,
        patch: &PersistenceConfigPatch,
    ) -> Result<(), ConfigError> {
        let text = std::fs::read_to_string(path)?;
        let mut document = text
            .parse::<toml_edit::DocumentMut>()
            .map_err(|error| ConfigError::Policy(format!("invalid TOML: {error}")))?;
        if document.get("persistence").is_none() {
            document["persistence"] = toml_edit::Item::Table(toml_edit::Table::new());
        }
        let table = document["persistence"]
            .as_table_mut()
            .ok_or_else(|| ConfigError::Policy("persistence is not a table".into()))?;
        if let Some(value) = patch.history_enabled {
            table.insert("history_enabled", toml_edit::value(value));
        }
        if let Some(value) = patch.saved_document_max_bytes {
            table.insert(
                "saved_document_max_bytes",
                toml_edit::value(signed_integer(
                    value,
                    "persistence.saved_document_max_bytes",
                )?),
            );
        }
        if let Some(value) = patch.audit_retention_days {
            table.insert(
                "audit_retention_days",
                toml_edit::value(signed_integer(value, "persistence.audit_retention_days")?),
            );
        }
        if let Some(value) = patch.history_retention_days {
            table.insert(
                "history_retention_days",
                toml_edit::value(signed_integer(value, "persistence.history_retention_days")?),
            );
        }
        if let Some(value) = patch.cache_retention_days {
            table.insert(
                "cache_retention_days",
                toml_edit::value(signed_integer(value, "persistence.cache_retention_days")?),
            );
        }
        if let Some(value) = patch.document_cache_retention_days {
            table.insert(
                "document_cache_retention_days",
                toml_edit::value(signed_integer(
                    value,
                    "persistence.document_cache_retention_days",
                )?),
            );
        }
        if let Some(value) = patch.purge_interval_seconds {
            table.insert(
                "purge_interval_seconds",
                toml_edit::value(signed_integer(value, "persistence.purge_interval_seconds")?),
            );
        }
        if let Some(value) = patch.auto_backup_enabled {
            table.insert("auto_backup_enabled", toml_edit::value(value));
        }
        if let Some(value) = patch.auto_backup_interval_seconds {
            table.insert(
                "auto_backup_interval_seconds",
                toml_edit::value(signed_integer(
                    value,
                    "persistence.auto_backup_interval_seconds",
                )?),
            );
        }
        if let Some(value) = patch.auto_backup_max_count {
            table.insert(
                "auto_backup_max_count",
                toml_edit::value(signed_integer(value, "persistence.auto_backup_max_count")?),
            );
        }
        set_clearable_string(table, "backup_directory", patch.backup_directory.as_deref());
        std::fs::write(path, document.to_string())?;
        Ok(())
    }

    /// Write every field `patch` sets to `[telemetry]`, and nothing else.
    /// Same guarantees as [`Config::set_inference_fields`].
    pub fn set_telemetry_fields(
        path: &Path,
        patch: &TelemetryConfigPatch,
    ) -> Result<(), ConfigError> {
        let text = std::fs::read_to_string(path)?;
        let mut document = text
            .parse::<toml_edit::DocumentMut>()
            .map_err(|error| ConfigError::Policy(format!("invalid TOML: {error}")))?;
        if document.get("telemetry").is_none() {
            document["telemetry"] = toml_edit::Item::Table(toml_edit::Table::new());
        }
        let table = document["telemetry"]
            .as_table_mut()
            .ok_or_else(|| ConfigError::Policy("telemetry is not a table".into()))?;
        if let Some(value) = patch.persistence_enabled {
            table.insert("persistence_enabled", toml_edit::value(value));
        }
        if let Some(value) = patch.retention_days {
            table.insert(
                "retention_days",
                toml_edit::value(signed_integer(value, "telemetry.retention_days")?),
            );
        }
        std::fs::write(path, document.to_string())?;
        Ok(())
    }

    /// Write every field `patch` sets to `[deep]`, and nothing else —
    /// `extractor`, `renderer`, `ranking_v2` and `gaps` are separate nested
    /// tables this never touches (see [`DeepConfigPatch`]). Same guarantees
    /// as [`Config::set_inference_fields`].
    pub fn set_deep_fields(path: &Path, patch: &DeepConfigPatch) -> Result<(), ConfigError> {
        let text = std::fs::read_to_string(path)?;
        let mut document = text
            .parse::<toml_edit::DocumentMut>()
            .map_err(|error| ConfigError::Policy(format!("invalid TOML: {error}")))?;
        let table = Self::table_at(&mut document, &["deep"])?;
        if let Some(value) = patch.top_k {
            table.insert(
                "top_k",
                toml_edit::value(signed_integer(value, "deep.top_k")?),
            );
        }
        if let Some(value) = patch.max_fetches {
            table.insert(
                "max_fetches",
                toml_edit::value(signed_integer(value, "deep.max_fetches")?),
            );
        }
        if let Some(value) = patch.max_bytes {
            table.insert(
                "max_bytes",
                toml_edit::value(signed_integer(value, "deep.max_bytes")?),
            );
        }
        if let Some(value) = patch.max_redirects {
            table.insert(
                "max_redirects",
                toml_edit::value(signed_integer(value, "deep.max_redirects")?),
            );
        }
        if let Some(value) = patch.max_crawl_urls {
            table.insert(
                "max_crawl_urls",
                toml_edit::value(signed_integer(value, "deep.max_crawl_urls")?),
            );
        }
        if let Some(value) = patch.max_depth {
            table.insert(
                "max_depth",
                toml_edit::value(signed_integer(value, "deep.max_depth")?),
            );
        }
        if let Some(value) = patch.respect_robots {
            table.insert("respect_robots", toml_edit::value(value));
        }
        if let Some(value) = patch.robots_timeout_ms {
            table.insert(
                "robots_timeout_ms",
                toml_edit::value(signed_integer(value, "deep.robots_timeout_ms")?),
            );
        }
        if let Some(value) = patch.robots_max_bytes {
            table.insert(
                "robots_max_bytes",
                toml_edit::value(signed_integer(value, "deep.robots_max_bytes")?),
            );
        }
        if let Some(value) = patch.timeout_ms {
            table.insert(
                "timeout_ms",
                toml_edit::value(signed_integer(value, "deep.timeout_ms")?),
            );
        }
        std::fs::write(path, document.to_string())?;
        Ok(())
    }

    /// Write every field `patch` sets to `[deep.extractor]`, and nothing
    /// else. Same guarantees as [`Config::set_inference_fields`].
    pub fn set_deep_extractor_fields(
        path: &Path,
        patch: &ExtractorConfigPatch,
    ) -> Result<(), ConfigError> {
        let text = std::fs::read_to_string(path)?;
        let mut document = text
            .parse::<toml_edit::DocumentMut>()
            .map_err(|error| ConfigError::Policy(format!("invalid TOML: {error}")))?;
        let table = Self::table_at(&mut document, &["deep", "extractor"])?;
        if let Some(value) = &patch.executable {
            table.insert("executable", toml_edit::value(value.as_str()));
        }
        if let Some(value) = &patch.version {
            table.insert("version", toml_edit::value(value.as_str()));
        }
        if let Some(value) = patch.timeout_ms {
            table.insert(
                "timeout_ms",
                toml_edit::value(signed_integer(value, "deep.extractor.timeout_ms")?),
            );
        }
        if let Some(value) = patch.max_output_bytes {
            table.insert(
                "max_output_bytes",
                toml_edit::value(signed_integer(value, "deep.extractor.max_output_bytes")?),
            );
        }
        std::fs::write(path, document.to_string())?;
        Ok(())
    }

    /// Write every field `patch` sets to `[deep.renderer]`, and nothing
    /// else. Same guarantees as [`Config::set_inference_fields`]; see
    /// [`RendererConfigPatch`] for the `isolated`-profile interaction the
    /// caller's pre-write validation is expected to catch.
    pub fn set_deep_renderer_fields(
        path: &Path,
        patch: &RendererConfigPatch,
    ) -> Result<(), ConfigError> {
        let text = std::fs::read_to_string(path)?;
        let mut document = text
            .parse::<toml_edit::DocumentMut>()
            .map_err(|error| ConfigError::Policy(format!("invalid TOML: {error}")))?;
        let table = Self::table_at(&mut document, &["deep", "renderer"])?;
        if let Some(value) = patch.enabled {
            table.insert("enabled", toml_edit::value(value));
        }
        if let Some(value) = patch.max_browser_calls {
            table.insert(
                "max_browser_calls",
                toml_edit::value(signed_integer(value, "deep.renderer.max_browser_calls")?),
            );
        }
        if let Some(value) = patch.timeout_ms {
            table.insert(
                "timeout_ms",
                toml_edit::value(signed_integer(value, "deep.renderer.timeout_ms")?),
            );
        }
        if let Some(value) = patch.shutdown_grace_ms {
            table.insert(
                "shutdown_grace_ms",
                toml_edit::value(signed_integer(value, "deep.renderer.shutdown_grace_ms")?),
            );
        }
        if let Some(value) = patch.max_memory_mb {
            table.insert(
                "max_memory_mb",
                toml_edit::value(signed_integer(value, "deep.renderer.max_memory_mb")?),
            );
        }
        if let Some(value) = patch.max_redirects {
            table.insert(
                "max_redirects",
                toml_edit::value(signed_integer(value, "deep.renderer.max_redirects")?),
            );
        }
        if let Some(value) = &patch.sandbox_path {
            table.insert("sandbox_path", toml_edit::value(value.as_str()));
        }
        if let Some(value) = patch.max_dom_bytes {
            table.insert(
                "max_dom_bytes",
                toml_edit::value(signed_integer(value, "deep.renderer.max_dom_bytes")?),
            );
        }
        std::fs::write(path, document.to_string())?;
        Ok(())
    }

    /// Navigate `document` to the (possibly nested) table at `path`,
    /// creating any missing intermediate table along the way. Shared by
    /// every field-level setter that writes into a nested section (`[deep]`,
    /// `[deep.extractor]`, `[deep.renderer]`, …) so each one only states its
    /// own path instead of re-deriving this walk.
    fn table_at<'a>(
        document: &'a mut toml_edit::DocumentMut,
        path: &[&str],
    ) -> Result<&'a mut toml_edit::Table, ConfigError> {
        let mut current = document.as_table_mut();
        for segment in path {
            if current.get(segment).is_none() {
                current.insert(segment, toml_edit::Item::Table(toml_edit::Table::new()));
            }
            current = current[segment]
                .as_table_mut()
                .ok_or_else(|| ConfigError::Policy(format!("{segment} is not a table")))?;
        }
        Ok(current)
    }

    /// Write every field `patch` sets to `[server]` (and `[server.tls]` for
    /// the two TLS fields), and nothing else — `clients` is never touched
    /// here (see [`ServerConfigPatch`]). Same guarantees as
    /// [`Config::set_inference_fields`]: the caller validates a candidate
    /// first; this only performs the write. Writing does **not** mean every
    /// field takes effect without a restart — see [`ServerConfigPatch`] and
    /// [`ReloadKind`].
    pub fn set_server_fields(path: &Path, patch: &ServerConfigPatch) -> Result<(), ConfigError> {
        let text = std::fs::read_to_string(path)?;
        let mut document = text
            .parse::<toml_edit::DocumentMut>()
            .map_err(|error| ConfigError::Policy(format!("invalid TOML: {error}")))?;
        {
            let table = Self::table_at(&mut document, &["server"])?;
            if let Some(value) = &patch.bind {
                table.insert("bind", toml_edit::value(value.as_str()));
            }
            if let Some(value) = patch.port {
                table.insert("port", toml_edit::value(i64::from(value)));
            }
            if let Some(value) = &patch.token_env {
                table.insert("token_env", toml_edit::value(value.as_str()));
            }
            if let Some(value) = patch.no_auth {
                table.insert("no_auth", toml_edit::value(value));
            }
            if let Some(value) = &patch.allowed_hosts {
                let mut hosts = toml_edit::Array::new();
                for host in value {
                    hosts.push(host.as_str());
                }
                table.insert("allowed_hosts", toml_edit::value(hosts));
            }
            if let Some(value) = &patch.allowed_origins {
                let mut origins = toml_edit::Array::new();
                for origin in value {
                    origins.push(origin.as_str());
                }
                table.insert("allowed_origins", toml_edit::value(origins));
            }
            if let Some(value) = patch.max_body_bytes {
                table.insert(
                    "max_body_bytes",
                    toml_edit::value(signed_integer(value, "server.max_body_bytes")?),
                );
            }
            if let Some(value) = patch.max_header_bytes {
                table.insert(
                    "max_header_bytes",
                    toml_edit::value(signed_integer(value, "server.max_header_bytes")?),
                );
            }
            if let Some(value) = patch.request_timeout_ms {
                table.insert(
                    "request_timeout_ms",
                    toml_edit::value(signed_integer(value, "server.request_timeout_ms")?),
                );
            }
            if let Some(value) = patch.idle_timeout_ms {
                table.insert(
                    "idle_timeout_ms",
                    toml_edit::value(signed_integer(value, "server.idle_timeout_ms")?),
                );
            }
            if let Some(value) = patch.rate_limit_per_minute {
                table.insert(
                    "rate_limit_per_minute",
                    toml_edit::value(signed_integer(value, "server.rate_limit_per_minute")?),
                );
            }
            if let Some(value) = patch.max_connections {
                table.insert(
                    "max_connections",
                    toml_edit::value(signed_integer(value, "server.max_connections")?),
                );
            }
        }
        if patch.tls_cert_path.is_some() || patch.tls_key_path.is_some() {
            let table = Self::table_at(&mut document, &["server", "tls"])?;
            set_clearable_string(table, "cert_path", patch.tls_cert_path.as_deref());
            set_clearable_string(table, "key_path", patch.tls_key_path.as_deref());
        }
        std::fs::write(path, document.to_string())?;
        Ok(())
    }

    /// Insert a new `[providers.<name>]` governance record, or replace the
    /// one declared for `name`, and nothing else.
    ///
    /// Same guarantees as `Config::set_scalar_field`: comments survive;
    /// the caller is expected to have already validated a candidate config
    /// with this record applied. This is the *ficha* (`approval_status`,
    /// `reviewer`, `terms_url`, …) — it never touches `providers.enabled`,
    /// which stays [`Config::set_provider_enabled`]'s job, so approving a
    /// source's paperwork here never silently turns its traffic on.
    pub fn upsert_provider_record(
        path: &Path,
        name: &str,
        record: &ProviderRuntimeConfig,
    ) -> Result<(), ConfigError> {
        if !is_provider_key(name) {
            return Err(ConfigError::Policy(format!(
                "invalid provider name: {name}"
            )));
        }
        let text = std::fs::read_to_string(path)?;
        let mut document = text
            .parse::<toml_edit::DocumentMut>()
            .map_err(|error| ConfigError::Policy(format!("invalid TOML: {error}")))?;
        if document.get("providers").is_none() {
            document["providers"] = toml_edit::Item::Table(toml_edit::Table::new());
        }
        let providers_table = document["providers"]
            .as_table_mut()
            .ok_or_else(|| ConfigError::Policy("providers is not a table".into()))?;
        providers_table.insert(
            name,
            toml_edit::Item::Table(provider_record_to_table(record)),
        );
        std::fs::write(path, document.to_string())?;
        Ok(())
    }

    /// Replace `[ranking_policy]` wholesale with `policy`, and nothing else.
    /// See `Config::replace_table` for what "wholesale" means here.
    pub fn set_ranking_policy(path: &Path, policy: &RankingPolicyV1) -> Result<(), ConfigError> {
        Self::replace_table(path, &["ranking_policy"], policy)
    }

    /// Replace `[diversity_policy]` wholesale. See
    /// [`Config::set_ranking_policy`].
    ///
    /// The caller must validate a candidate first: `diversity_policy` and
    /// `search_policy` share three limits that `Config::validate` requires
    /// to agree (see the cross-check there), so replacing one alone can
    /// make the pair invalid even though each is individually well-formed.
    pub fn set_diversity_policy(
        path: &Path,
        policy: &DiversityPolicyV1,
    ) -> Result<(), ConfigError> {
        Self::replace_table(path, &["diversity_policy"], policy)
    }

    /// Replace `[search_policy]` wholesale. See
    /// [`Config::set_diversity_policy`] for the cross-check with
    /// `diversity_policy` this must still satisfy.
    pub fn set_search_policy(path: &Path, policy: &SearchPolicyV1) -> Result<(), ConfigError> {
        Self::replace_table(path, &["search_policy"], policy)
    }

    /// Replace `[deep.ranking_v2.policy]` wholesale. See
    /// [`Config::set_ranking_policy`].
    pub fn set_ranking_v2_policy(path: &Path, policy: &RankingV2Policy) -> Result<(), ConfigError> {
        Self::replace_table(path, &["deep", "ranking_v2", "policy"], policy)
    }

    /// Replace `[deep.gaps.policy]` wholesale. See
    /// [`Config::set_ranking_policy`].
    pub fn set_gap_policy(path: &Path, policy: &GapPolicyV1) -> Result<(), ConfigError> {
        Self::replace_table(path, &["deep", "gaps", "policy"], policy)
    }

    /// Replace the table at the (possibly nested) `path` with the TOML
    /// serialization of `value`, and nothing else — every other section, and
    /// every comment outside the replaced table, is untouched.
    ///
    /// The comment block immediately above `[section]`, if any, survives
    /// (it reads as documenting the section, so it carries over onto the
    /// replacement); unlike `Config::set_scalar_field`, comments *inside*
    /// the table do not — the whole table is swapped for a fresh
    /// serialization of `value`, not merged key by key. That trade-off fits
    /// the search-quality policies this backs — an admin-scoped caller
    /// fetches the current policy in full, edits it, and sends the whole
    /// object back, so there is no "one field at a time" comment to
    /// preserve the way there is for `answer.enabled` or a single provider
    /// field. The caller is expected to have already validated a candidate
    /// config with this change applied, exactly as with every other setter
    /// here.
    fn replace_table<T: Serialize>(
        path: &Path,
        section: &[&str],
        value: &T,
    ) -> Result<(), ConfigError> {
        let text = std::fs::read_to_string(path)?;
        let mut document = text
            .parse::<toml_edit::DocumentMut>()
            .map_err(|error| ConfigError::Policy(format!("invalid TOML: {error}")))?;
        let fragment = toml::to_string(value)
            .map_err(|error| ConfigError::Policy(format!("failed to serialize policy: {error}")))?;
        let fragment_document = fragment
            .parse::<toml_edit::DocumentMut>()
            .map_err(|error| ConfigError::Policy(format!("failed to render policy: {error}")))?;
        let mut table = fragment_document.as_table().clone();
        let mut current = document.as_table_mut();
        for segment in &section[..section.len() - 1] {
            if current.get(segment).is_none() {
                current.insert(segment, toml_edit::Item::Table(toml_edit::Table::new()));
            }
            current = current[segment]
                .as_table_mut()
                .ok_or_else(|| ConfigError::Policy(format!("{segment} is not a table")))?;
        }
        let leaf = section[section.len() - 1];
        // A comment immediately above `[section]` reads as documenting the
        // section, not "inside" it — carry that leading decor over from the
        // table being replaced so it survives, same as every other setter's
        // surrounding comments do.
        if let Some(existing) = current.get(leaf).and_then(toml_edit::Item::as_table) {
            *table.decor_mut() = existing.decor().clone();
        }
        current.insert(leaf, toml_edit::Item::Table(table));
        std::fs::write(path, document.to_string())?;
        Ok(())
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
        self.validate_answer()?;
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
        // Every numeric limit is bounded in *both* directions, unconditionally,
        // even when the feature that uses it is currently off: the value is
        // still written to the file as an i64, and an unbounded one (e.g.
        // `usize::MAX`) would wrap negative on write and poison the file for
        // the next reload or restart. The ceilings below are deliberately
        // generous — far above any real deployment, far below `i64::MAX`.
        if self.persistence.purge_interval_seconds > 30 * 86_400
            || (self.persistence.purge_interval_seconds > 0
                && self.persistence.purge_interval_seconds < 60)
        {
            return Err(ConfigError::Policy(
                "persistence.purge_interval_seconds must be 0 (disabled) or between 60 and 2592000 (30 days)".into(),
            ));
        }
        if self.persistence.auto_backup_interval_seconds > 30 * 86_400 {
            return Err(ConfigError::Policy(
                "persistence.auto_backup_interval_seconds must be at most 2592000 (30 days)".into(),
            ));
        }
        if self.persistence.auto_backup_max_count > 365 {
            return Err(ConfigError::Policy(
                "persistence.auto_backup_max_count must be at most 365".into(),
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
        // Robots retrieval limits are bounded unconditionally (see the note on
        // `persistence` above): the values are written to the file even while
        // `respect_robots` is off, and must never wrap negative on write.
        if !(100..=30_000).contains(&self.deep.robots_timeout_ms)
            || !(1_024..=1_048_576).contains(&self.deep.robots_max_bytes)
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
        // Deep safety limits, bounded in both directions (see the note on
        // `persistence` above). `max_redirects`/`max_dom_bytes` had no check
        // at all before; everything here is written to the file as an i64 and
        // must stay far below `i64::MAX`.
        if !(1..=1_000).contains(&self.deep.top_k)
            || !(1..=1_000).contains(&self.deep.max_fetches)
            || !(1..=4 * 1024 * 1024 * 1024).contains(&self.deep.max_bytes)
            || !(0..=100).contains(&self.deep.max_redirects)
            || !(1..=10_000).contains(&self.deep.max_crawl_urls)
            || self.deep.max_depth > 2
            || !(1..=86_400_000).contains(&self.deep.timeout_ms)
            || !(1..=86_400_000).contains(&self.deep.extractor.timeout_ms)
            || !(1..=4 * 1024 * 1024 * 1024).contains(&self.deep.extractor.max_output_bytes)
            || !(1..=1_000_000).contains(&self.deep.renderer.max_browser_calls)
            || !(1..=86_400_000).contains(&self.deep.renderer.timeout_ms)
            || !(1..=86_400_000).contains(&self.deep.renderer.shutdown_grace_ms)
            || !(1..=1_048_576).contains(&self.deep.renderer.max_memory_mb)
            || !(0..=100).contains(&self.deep.renderer.max_redirects)
            || !(1..=4 * 1024 * 1024 * 1024).contains(&self.deep.renderer.max_dom_bytes)
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
            || self.server.request_timeout_ms > 86_400_000
            || self.server.idle_timeout_ms == 0
            || self.server.idle_timeout_ms > 86_400_000
            || self.server.rate_limit_per_minute == 0
            || self.server.rate_limit_per_minute > 1_000_000
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
        }
        // Every numeric inference limit is bounded unconditionally, even while
        // `data_policy.inference` is `disabled`/`local_only`: the values are
        // still written to the file as an i64, and an unbounded one would wrap
        // negative on write and poison the file for the next reload or restart.
        // The ceilings are deliberately generous — far above any real
        // deployment, far below `i64::MAX`.
        if !(crate::inference::MINIMUM_EMBEDDING_DIMENSIONS
            ..=crate::inference::MAXIMUM_EMBEDDING_DIMENSIONS)
            .contains(&self.inference.embedding_dimensions)
            || !(1..=1_000_000).contains(&self.inference.max_documents)
            || !(1..=100_000_000).contains(&self.inference.max_input_chars)
            || !(1..=1_000_000).contains(&self.inference.local_model_batch)
            || !(1..=crate::inference::MAXIMUM_REMOTE_BATCH)
                .contains(&self.inference.remote_max_batch)
            || !(100..=60_000).contains(&self.inference.remote_timeout_ms)
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

    /// Same governance as `validate_inference`'s remote block, for the same
    /// reason: `answer` is a second, independent kind of remote model call
    /// (chat completions, not embeddings), so it gets its own endpoint,
    /// credential and bounded limits rather than silently reusing
    /// `inference.remote_*` for a different contract.
    fn validate_answer(&self) -> Result<(), ConfigError> {
        // Numeric limits are bounded unconditionally, even while the feature
        // is off: they are still written to the file as an i64, and an
        // unbounded value would wrap negative on write (see
        // `validate_inference`). `max_sources` keeps its documented `<= 32`
        // operating bound; the others get generous ceilings far below
        // `i64::MAX`.
        if !(100..=60_000).contains(&self.answer.timeout_ms)
            || !(1..=32).contains(&self.answer.max_sources)
            || !(1..=10_000_000).contains(&self.answer.max_source_chars)
            || !(1..=1_000_000).contains(&self.answer.max_answer_tokens)
        {
            return Err(ConfigError::Policy("invalid answer limit".into()));
        }
        if !self.answer.enabled {
            return Ok(());
        }
        if !self.data_policy.allows_remote_inference() {
            return Err(ConfigError::Policy(
                "answer.enabled requires a standard profile, governed egress and \
                 data_policy.inference = \"remote_explicit\""
                    .into(),
            ));
        }
        let endpoint =
            self.answer.endpoint.as_deref().ok_or_else(|| {
                ConfigError::Policy("answer.enabled requires answer.endpoint".into())
            })?;
        crate::inference::validate_remote_endpoint(endpoint)
            .map_err(|error| ConfigError::Policy(error.to_string()))?;
        if self.answer.model.as_deref().is_none_or(str::is_empty) {
            return Err(ConfigError::Policy(
                "answer.enabled requires answer.model".into(),
            ));
        }
        Ok(())
    }
}

/// Whether writing a field to the config file and asking the running
/// process to reload (`Config::load_optional` + a service rebuild — see
/// `AppState::reload` in `amatl-server`) is enough to apply the change, or
/// whether the process must be restarted because that value is only read
/// once, at process startup.
///
/// Every section of [`Config`] other than `server` is rebuilt in full on
/// every reload (`AmatlService::reloaded_detached`), so [`ReloadKind::of`]
/// defaults to [`ReloadKind::Hot`]. Within `server`, most fields *are*
/// re-read on reload too — `AppState::reload` recomputes the security
/// snapshot (host/origin allowlists, header/body ceilings, request timeout,
/// rate limit, TLS flag, and the client/credential set) from the freshly
/// loaded config on every call. What a reload cannot change is the listener
/// itself and the axum layers wired onto it once, in
/// `build_router_with_reload`:
/// - `server.bind`, `server.port`: the TCP socket is bound once in
///   `serve_with_config_path`.
/// - `server.tls.cert_path`, `server.tls.key_path`: the TLS acceptor is
///   built from the same call.
/// - `server.max_connections`: `ConcurrencyLimitLayer` is constructed once
///   and never swapped.
/// - `server.max_body_bytes`: `DefaultBodyLimit` is also a fixed layer.
///   Lowering it still takes effect (the reloaded security snapshot
///   double-checks the new, smaller ceiling in `security_middleware`), but
///   *raising* it does nothing until restart, because the original,
///   smaller layer limit still rejects the request first. Classified
///   `Cold` here because a caller that only checks this classification
///   should never advertise a change as fully applied when it may not be.
/// - `server.idle_timeout_ms`: read once in `serve_with_config_path` and
///   baked into the HTTP/1 and HTTP/2 keep-alive/header-read timers passed
///   to `axum_server`'s builder; a reload never touches that builder.
///
/// This exists so an admin endpoint can check the classification instead of
/// re-deriving this judgment — see `PATCH /server/pending-config` in
/// `amatl-server`, which reports exactly this per field it writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReloadKind {
    /// Applied by a reload; no restart required.
    Hot,
    /// Only takes full effect the next time the process starts.
    Cold,
}

impl ReloadKind {
    /// Classify `[section].key` (or, for a nested field, the dotted path
    /// e.g. `"server.tls.cert_path"`).
    pub fn of(section: &str, key: &str) -> Self {
        match (section, key) {
            (
                "server",
                "bind" | "port" | "max_connections" | "max_body_bytes" | "idle_timeout_ms",
            ) => Self::Cold,
            ("server.tls", "cert_path" | "key_path") => Self::Cold,
            _ => Self::Hot,
        }
    }
}

/// Tool names the MCP surface exposes, and the vocabulary a client allowlist
/// may use. Kept here so an unknown name is rejected by configuration
/// validation instead of silently granting nothing.
pub const MCP_TOOLS: [&str; 6] = [
    "search",
    "deep_search",
    "fetch",
    "providers",
    "status",
    "answer",
];

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

/// Render one [`ServerClient`] as the `toml_edit` table
/// [`Config::upsert_server_client`] writes into `[[server.clients]]`.
///
/// Only fields that are `Some`/non-empty are written, matching how an
/// operator would hand-author the same entry: an absent `expires_at` means
/// "never expires", not an explicit null.
fn server_client_to_table(client: &ServerClient) -> toml_edit::Table {
    let mut table = toml_edit::Table::new();
    table.insert("id", toml_edit::value(client.id.as_str()));
    if let Some(token_env) = client.token_env.as_deref() {
        table.insert("token_env", toml_edit::value(token_env));
    }
    if let Some(token_sha256) = client.token_sha256.as_deref() {
        table.insert("token_sha256", toml_edit::value(token_sha256));
    }
    if let Some(expires_at) = client.expires_at.as_deref() {
        table.insert("expires_at", toml_edit::value(expires_at));
    }
    let mut scopes = toml_edit::Array::new();
    for scope in &client.scopes {
        scopes.push(scope.as_str());
    }
    table.insert("scopes", toml_edit::value(scopes));
    let mut tools = toml_edit::Array::new();
    for tool in &client.tools {
        tools.push(tool.as_str());
    }
    table.insert("tools", toml_edit::value(tools));
    table
}

/// Set `table[key]` to `value` when it's non-empty, or remove the key
/// entirely when it's `Some("")` — `clearable`'s disk-writing counterpart.
/// `None` (the field was absent from the patch) leaves the key untouched
/// either way.
fn set_clearable_string(table: &mut toml_edit::Table, key: &str, value: Option<&str>) {
    if let Some(value) = value {
        if value.is_empty() {
            table.remove(key);
        } else {
            table.insert(key, toml_edit::value(value));
        }
    }
}

/// Render one [`ProviderRuntimeConfig`] as the `toml_edit` table
/// [`Config::upsert_provider_record`] writes into `[providers.<name>]`.
///
/// Only fields that are `Some`/non-empty are written, matching how every
/// hand-authored ficha in the repository already looks (see
/// `docs/gobernanza-providers.md`): an absent field reads as "not on file",
/// not as an explicit empty value.
fn provider_record_to_table(record: &ProviderRuntimeConfig) -> toml_edit::Table {
    let mut table = toml_edit::Table::new();
    if let Some(value) = record.adapter_version.as_deref() {
        table.insert("adapter_version", toml_edit::value(value));
    }
    table.insert(
        "approval_status",
        toml_edit::value(record.approval_status.as_str()),
    );
    if let Some(value) = record.reviewed_at.as_deref() {
        table.insert("reviewed_at", toml_edit::value(value));
    }
    if let Some(value) = record.reviewer.as_deref() {
        table.insert("reviewer", toml_edit::value(value));
    }
    if let Some(value) = record.terms_url.as_deref() {
        table.insert("terms_url", toml_edit::value(value));
    }
    if let Some(value) = record.terms_version_or_date.as_deref() {
        table.insert("terms_version_or_date", toml_edit::value(value));
    }
    if let Some(value) = record.allowed_access_method.as_deref() {
        table.insert("allowed_access_method", toml_edit::value(value));
    }
    if let Some(value) = record.plan_or_contract.as_deref() {
        table.insert("plan_or_contract", toml_edit::value(value));
    }
    if let Some(value) = record.rate_limit.as_deref() {
        table.insert("rate_limit", toml_edit::value(value));
    }
    if let Some(value) = record.cost_model.as_deref() {
        table.insert("cost_model", toml_edit::value(value));
    }
    if let Some(value) = record.credential_env.as_deref() {
        table.insert("credential_env", toml_edit::value(value));
    }
    table.insert("storage_rights", toml_edit::value(record.storage_rights));
    if !record.supported_regions.is_empty() {
        let mut regions = toml_edit::Array::new();
        for region in &record.supported_regions {
            regions.push(region.as_str());
        }
        table.insert("supported_regions", toml_edit::value(regions));
    }
    if !record.supported_filters.is_empty() {
        let mut filters = toml_edit::Array::new();
        for filter in &record.supported_filters {
            filters.push(filter.as_str());
        }
        table.insert("supported_filters", toml_edit::value(filters));
    }
    if let Some(value) = record.data_handling_notes.as_deref() {
        table.insert("data_handling_notes", toml_edit::value(value));
    }
    if let Some(value) = record.operational_risk.as_deref() {
        table.insert("operational_risk", toml_edit::value(value));
    }
    table
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

    /// Locks in a policy decision, not just a paperwork gap: Brave and
    /// Mojeek are `Rejected`, with the reason stated in `cost_model`/
    /// `operational_risk`, precisely so a future review — human or
    /// automated — reads a closed "no" instead of an incomplete dossier
    /// inviting completion. If this regresses to `Draft`, that decision was
    /// silently lost.
    #[test]
    fn paid_providers_are_rejected_by_default_not_merely_draft() {
        let config = Config::default();
        for name in ["brave", "mojeek"] {
            let record = config.providers.get(name).unwrap();
            assert_eq!(
                record.approval_status,
                ApprovalStatus::Rejected,
                "{name} must stay Rejected, not Draft: it requires a paid plan and \
                 operator policy excludes paid providers"
            );
            assert!(
                !record.approved(),
                "{name} must never pass the approval gate"
            );
            assert!(
                record
                    .operational_risk
                    .as_deref()
                    .is_some_and(|value| value.contains("operator policy")),
                "{name}'s rejection reason must stay explicit, not silently blanked"
            );
        }
        // The two free sources stay eligible for approval — only their
        // operator-specific identity/date fields are missing, not the
        // decision itself.
        for name in ["searxng", "marginalia"] {
            assert_eq!(
                config.providers.get(name).unwrap().approval_status,
                ApprovalStatus::Draft
            );
        }
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
            vec!["brave", "custom_archive", "marginalia", "mojeek", "searxng"]
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
    fn unbounded_patch_values_are_rejected_by_validation() {
        // Regression for the overflow corruption: numeric patch fields only
        // checked `!= 0` (or a bare minimum), so `usize::MAX`/`u64::MAX`
        // passed validation, were cast `as i64` on write (wrapping to -1),
        // and poisoned the file for the next reload or restart. Every field
        // below is written to the file as an i64, so every one must be
        // bounded — even while the feature that uses it is currently off.
        let mut inference_documents = Config::default();
        inference_documents.inference.max_documents = usize::MAX;
        assert!(inference_documents.validate().is_err());

        let mut inference_chars = Config::default();
        inference_chars.inference.max_input_chars = usize::MAX;
        assert!(inference_chars.validate().is_err());

        let mut inference_batch = Config::default();
        inference_batch.inference.local_model_batch = usize::MAX;
        assert!(inference_batch.validate().is_err());

        let mut remote_timeout = Config::default();
        remote_timeout.inference.remote_timeout_ms = u64::MAX;
        assert!(remote_timeout.validate().is_err());

        let mut remote_batch = Config::default();
        remote_batch.inference.remote_max_batch = usize::MAX;
        assert!(remote_batch.validate().is_err());

        let mut answer_timeout = Config::default();
        answer_timeout.answer.timeout_ms = u64::MAX;
        assert!(answer_timeout.validate().is_err());

        let mut answer_chars = Config::default();
        answer_chars.answer.max_source_chars = usize::MAX;
        assert!(answer_chars.validate().is_err());

        let mut answer_tokens = Config::default();
        answer_tokens.answer.max_answer_tokens = u32::MAX;
        assert!(answer_tokens.validate().is_err());

        let mut purge = Config::default();
        purge.persistence.purge_interval_seconds = u64::MAX;
        assert!(purge.validate().is_err());

        let mut backup_interval = Config::default();
        backup_interval.persistence.auto_backup_interval_seconds = u64::MAX;
        assert!(backup_interval.validate().is_err());

        let mut backup_count = Config::default();
        backup_count.persistence.auto_backup_max_count = u32::MAX;
        assert!(backup_count.validate().is_err());

        let mut deep_bytes = Config::default();
        deep_bytes.deep.max_bytes = u64::MAX;
        assert!(deep_bytes.validate().is_err());

        let mut deep_timeout = Config::default();
        deep_timeout.deep.timeout_ms = u64::MAX;
        assert!(deep_timeout.validate().is_err());

        let mut robots_timeout = Config::default();
        robots_timeout.deep.robots_timeout_ms = u64::MAX;
        assert!(robots_timeout.validate().is_err());

        let mut extractor_timeout = Config::default();
        extractor_timeout.deep.extractor.timeout_ms = u64::MAX;
        assert!(extractor_timeout.validate().is_err());

        let mut renderer_timeout = Config::default();
        renderer_timeout.deep.renderer.timeout_ms = u64::MAX;
        assert!(renderer_timeout.validate().is_err());

        let mut renderer_dom = Config::default();
        renderer_dom.deep.renderer.max_dom_bytes = u64::MAX;
        assert!(renderer_dom.validate().is_err());

        let mut request_timeout = Config::default();
        request_timeout.server.request_timeout_ms = u64::MAX;
        assert!(request_timeout.validate().is_err());

        let mut idle_timeout = Config::default();
        idle_timeout.server.idle_timeout_ms = u64::MAX;
        assert!(idle_timeout.validate().is_err());

        let mut rate_limit = Config::default();
        rate_limit.server.rate_limit_per_minute = u32::MAX;
        assert!(rate_limit.validate().is_err());
    }

    #[test]
    fn set_inference_fields_rejects_an_unstorable_value_without_touching_the_file() {
        // Last line of defense for the same corruption: even a caller that
        // skips `Config::validate` must not be able to write a value that
        // wraps negative (`usize::MAX as i64` is `-1`). The file is left
        // exactly as it was.
        let path = std::env::temp_dir().join(format!(
            "amatl-config-inference-overflow-test-{}.toml",
            std::process::id()
        ));
        std::fs::write(
            &path,
            "schema_version = \"1\"\n\n[inference]\nmax_documents = 64\n",
        )
        .unwrap();

        let patch = InferenceConfigPatch {
            max_documents: Some(usize::MAX),
            ..Default::default()
        };
        let error = Config::set_inference_fields(&path, &patch)
            .unwrap_err()
            .to_string();
        assert!(error.contains("max_documents"), "{error}");

        let written = std::fs::read_to_string(&path).unwrap();
        std::fs::remove_file(&path).ok();
        assert!(written.contains("max_documents = 64"), "{written}");
        assert!(
            !written.contains("-1"),
            "file must never be corrupted: {written}"
        );
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

    /// The one config mutation a running server ever makes to its own file
    /// touches exactly one key and leaves every comment an operator wrote —
    /// including the ones documenting *other* fields — untouched.
    #[test]
    fn set_answer_enabled_flips_only_that_key_and_keeps_comments() {
        let path = std::env::temp_dir().join(format!(
            "amatl-config-toggle-test-{}.toml",
            std::process::id()
        ));
        std::fs::write(
            &path,
            "schema_version = \"1\"\n\n\
             # A comment an operator wrote about something unrelated.\n\
             [server]\n\
             port = 8080\n\n\
             [answer]\n\
             enabled = false\n\
             model = \"deepseek-ai/DeepSeek-V3\"\n",
        )
        .unwrap();

        Config::set_answer_enabled(&path, true).unwrap();
        let written = std::fs::read_to_string(&path).unwrap();
        std::fs::remove_file(&path).ok();

        assert!(written.contains("enabled = true"));
        assert!(written.contains("# A comment an operator wrote about something unrelated."));
        assert!(written.contains("model = \"deepseek-ai/DeepSeek-V3\""));
        assert!(written.contains("port = 8080"));
        let reparsed = Config::from_toml(&written).unwrap();
        assert!(reparsed.answer.enabled);
        assert_eq!(reparsed.server.port, 8080);
    }

    #[test]
    fn set_answer_enabled_creates_the_table_when_absent() {
        let path = std::env::temp_dir().join(format!(
            "amatl-config-toggle-notable-test-{}.toml",
            std::process::id()
        ));
        std::fs::write(&path, "schema_version = \"1\"\n").unwrap();

        Config::set_answer_enabled(&path, true).unwrap();
        let written = std::fs::read_to_string(&path).unwrap();
        std::fs::remove_file(&path).ok();

        let reparsed = Config::from_toml(&written).unwrap();
        assert!(reparsed.answer.enabled);
    }

    #[test]
    fn set_provider_enabled_adds_and_removes_the_name_only() {
        let path = std::env::temp_dir().join(format!(
            "amatl-config-provider-toggle-test-{}.toml",
            std::process::id()
        ));
        std::fs::write(
            &path,
            "schema_version = \"1\"\n\n\
             # A comment an operator wrote about something unrelated.\n\
             [providers]\n\
             enabled = [\"searxng\"]\n",
        )
        .unwrap();

        Config::set_provider_enabled(&path, "marginalia", true).unwrap();
        let written = std::fs::read_to_string(&path).unwrap();
        assert!(written.contains("# A comment an operator wrote about something unrelated."));
        let reparsed = Config::from_toml(&written).unwrap();
        assert_eq!(
            reparsed.providers.enabled,
            vec!["searxng".to_string(), "marginalia".to_string()]
        );

        // Enabling an already-enabled name does not duplicate it.
        Config::set_provider_enabled(&path, "marginalia", true).unwrap();
        let reparsed = Config::from_toml(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            reparsed.providers.enabled,
            vec!["searxng".to_string(), "marginalia".to_string()]
        );

        Config::set_provider_enabled(&path, "searxng", false).unwrap();
        let written = std::fs::read_to_string(&path).unwrap();
        std::fs::remove_file(&path).ok();
        let reparsed = Config::from_toml(&written).unwrap();
        assert_eq!(reparsed.providers.enabled, vec!["marginalia".to_string()]);
    }

    #[test]
    fn set_provider_enabled_creates_the_table_when_absent() {
        let path = std::env::temp_dir().join(format!(
            "amatl-config-provider-toggle-notable-test-{}.toml",
            std::process::id()
        ));
        std::fs::write(&path, "schema_version = \"1\"\n").unwrap();

        Config::set_provider_enabled(&path, "searxng", true).unwrap();
        let written = std::fs::read_to_string(&path).unwrap();
        std::fs::remove_file(&path).ok();

        let reparsed = Config::from_toml(&written).unwrap();
        assert_eq!(reparsed.providers.enabled, vec!["searxng".to_string()]);
    }

    #[test]
    fn set_provider_enabled_rejects_an_invalid_name() {
        let path = std::env::temp_dir().join(format!(
            "amatl-config-provider-toggle-invalid-{}.toml",
            std::process::id()
        ));
        std::fs::write(&path, "schema_version = \"1\"\n").unwrap();
        let result = Config::set_provider_enabled(&path, "Not Valid!", true);
        std::fs::remove_file(&path).ok();
        assert!(result.is_err());
    }

    #[test]
    fn reload_kind_flags_the_listener_level_server_fields_as_cold() {
        assert_eq!(ReloadKind::of("server", "bind"), ReloadKind::Cold);
        assert_eq!(ReloadKind::of("server", "port"), ReloadKind::Cold);
        assert_eq!(
            ReloadKind::of("server", "max_connections"),
            ReloadKind::Cold
        );
        assert_eq!(ReloadKind::of("server", "max_body_bytes"), ReloadKind::Cold);
        assert_eq!(
            ReloadKind::of("server", "idle_timeout_ms"),
            ReloadKind::Cold
        );
        assert_eq!(ReloadKind::of("server.tls", "cert_path"), ReloadKind::Cold);
        assert_eq!(ReloadKind::of("server.tls", "key_path"), ReloadKind::Cold);
    }

    #[test]
    fn reload_kind_defaults_to_hot() {
        // Fields amatl-server's `AppState::reload` re-derives every call.
        assert_eq!(ReloadKind::of("server", "allowed_hosts"), ReloadKind::Hot);
        assert_eq!(ReloadKind::of("server", "clients"), ReloadKind::Hot);
        assert_eq!(
            ReloadKind::of("server", "rate_limit_per_minute"),
            ReloadKind::Hot
        );
        // A whole other section, rebuilt in full by `reloaded_detached`.
        assert_eq!(ReloadKind::of("answer", "enabled"), ReloadKind::Hot);
        assert_eq!(ReloadKind::of("providers", "enabled"), ReloadKind::Hot);
        assert_eq!(ReloadKind::of("deep", "max_fetches"), ReloadKind::Hot);
    }

    fn sample_client(id: &str) -> ServerClient {
        ServerClient {
            id: id.to_string(),
            token_env: None,
            token_sha256: Some("a".repeat(64)),
            expires_at: None,
            scopes: vec![Scope::Read, Scope::Search],
            tools: vec![],
        }
    }

    #[test]
    fn upsert_server_client_appends_a_new_entry_and_keeps_comments() {
        let path = std::env::temp_dir().join(format!(
            "amatl-config-client-upsert-test-{}.toml",
            std::process::id()
        ));
        std::fs::write(
            &path,
            "schema_version = \"1\"\n\n\
             # A comment an operator wrote about something unrelated.\n\
             [server]\n\
             port = 8080\n",
        )
        .unwrap();

        Config::upsert_server_client(&path, &sample_client("dashboard")).unwrap();
        let written = std::fs::read_to_string(&path).unwrap();
        std::fs::remove_file(&path).ok();

        assert!(written.contains("# A comment an operator wrote about something unrelated."));
        assert!(written.contains("port = 8080"));
        let reparsed = Config::from_toml(&written).unwrap();
        assert_eq!(reparsed.server.clients.len(), 1);
        assert_eq!(reparsed.server.clients[0].id, "dashboard");
        assert_eq!(
            reparsed.server.clients[0].scopes,
            vec![Scope::Read, Scope::Search]
        );
        assert_eq!(
            reparsed.server.clients[0].token_sha256.as_deref(),
            Some("a".repeat(64).as_str())
        );
    }

    #[test]
    fn upsert_server_client_replaces_the_entry_with_the_same_id_only() {
        let path = std::env::temp_dir().join(format!(
            "amatl-config-client-upsert-replace-test-{}.toml",
            std::process::id()
        ));
        std::fs::write(&path, "schema_version = \"1\"\n").unwrap();

        Config::upsert_server_client(&path, &sample_client("dashboard")).unwrap();
        Config::upsert_server_client(&path, &sample_client("other")).unwrap();
        let mut replacement = sample_client("dashboard");
        replacement.scopes = vec![Scope::Admin];
        Config::upsert_server_client(&path, &replacement).unwrap();

        let written = std::fs::read_to_string(&path).unwrap();
        std::fs::remove_file(&path).ok();
        let reparsed = Config::from_toml(&written).unwrap();
        assert_eq!(reparsed.server.clients.len(), 2);
        let dashboard = reparsed
            .server
            .clients
            .iter()
            .find(|client| client.id == "dashboard")
            .unwrap();
        assert_eq!(dashboard.scopes, vec![Scope::Admin]);
        assert!(reparsed
            .server
            .clients
            .iter()
            .any(|client| client.id == "other"));
    }

    #[test]
    fn remove_server_client_drops_only_the_matching_id() {
        let path = std::env::temp_dir().join(format!(
            "amatl-config-client-remove-test-{}.toml",
            std::process::id()
        ));
        std::fs::write(&path, "schema_version = \"1\"\n").unwrap();
        Config::upsert_server_client(&path, &sample_client("dashboard")).unwrap();
        Config::upsert_server_client(&path, &sample_client("other")).unwrap();

        Config::remove_server_client(&path, "dashboard").unwrap();
        let written = std::fs::read_to_string(&path).unwrap();
        std::fs::remove_file(&path).ok();
        let reparsed = Config::from_toml(&written).unwrap();
        assert_eq!(reparsed.server.clients.len(), 1);
        assert_eq!(reparsed.server.clients[0].id, "other");
    }

    #[test]
    fn remove_server_client_is_a_no_op_when_absent() {
        let path = std::env::temp_dir().join(format!(
            "amatl-config-client-remove-absent-test-{}.toml",
            std::process::id()
        ));
        std::fs::write(&path, "schema_version = \"1\"\n").unwrap();
        let result = Config::remove_server_client(&path, "nobody");
        std::fs::remove_file(&path).ok();
        assert!(result.is_ok());
    }

    #[test]
    fn set_data_policy_fields_flip_only_that_key_and_keep_comments() {
        let path = std::env::temp_dir().join(format!(
            "amatl-config-data-policy-test-{}.toml",
            std::process::id()
        ));
        std::fs::write(
            &path,
            "schema_version = \"1\"\n\n\
             # A comment an operator wrote about something unrelated.\n\
             [data_policy]\n\
             profile = \"standard\"\n\
             egress = \"governed\"\n\
             inference = \"disabled\"\n",
        )
        .unwrap();

        Config::set_data_policy_profile(&path, SecurityProfile::Isolated).unwrap();
        Config::set_data_policy_egress(&path, EgressPolicy::Deny).unwrap();
        let written = std::fs::read_to_string(&path).unwrap();
        std::fs::remove_file(&path).ok();

        assert!(written.contains("# A comment an operator wrote about something unrelated."));
        let reparsed = Config::from_toml(&written).unwrap();
        assert_eq!(reparsed.data_policy.profile, SecurityProfile::Isolated);
        assert_eq!(reparsed.data_policy.egress, EgressPolicy::Deny);
        // Untouched by either call.
        assert_eq!(reparsed.data_policy.inference, InferenceMode::Disabled);
    }

    #[test]
    fn set_data_policy_inference_creates_the_table_when_absent() {
        let path = std::env::temp_dir().join(format!(
            "amatl-config-data-policy-notable-test-{}.toml",
            std::process::id()
        ));
        std::fs::write(&path, "schema_version = \"1\"\n").unwrap();

        Config::set_data_policy_inference(&path, InferenceMode::LocalOnly).unwrap();
        let written = std::fs::read_to_string(&path).unwrap();
        std::fs::remove_file(&path).ok();

        let reparsed = Config::from_toml(&written).unwrap();
        assert_eq!(reparsed.data_policy.inference, InferenceMode::LocalOnly);
    }

    #[test]
    fn data_policy_patch_writes_all_requested_fields_in_one_edit() {
        // `profile`, `egress` and `inference` cross-check each other in
        // `Config::validate`, so the HTTP endpoint must write them in a
        // single edit, never three independent writes that could leave the
        // file in a mix the next reload would reject.
        let path = std::env::temp_dir().join(format!(
            "amatl-config-data-policy-patch-test-{}.toml",
            std::process::id()
        ));
        std::fs::write(
            &path,
            "schema_version = \"1\"\n\n[data_policy]\nprofile = \"standard\"\negress = \"governed\"\ninference = \"disabled\"\n",
        )
        .unwrap();

        let patch = DataPolicyConfigPatch {
            profile: Some(SecurityProfile::Isolated),
            egress: Some(EgressPolicy::Deny),
            inference: Some(InferenceMode::LocalOnly),
        };
        let mut candidate = Config::from_toml(&std::fs::read_to_string(&path).unwrap()).unwrap();
        patch.apply(&mut candidate.data_policy);
        assert!(candidate.validate().is_ok());

        Config::set_data_policy_fields(&path, &patch).unwrap();
        let written = std::fs::read_to_string(&path).unwrap();
        std::fs::remove_file(&path).ok();

        let reparsed = Config::from_toml(&written).unwrap();
        assert_eq!(reparsed.data_policy.profile, SecurityProfile::Isolated);
        assert_eq!(reparsed.data_policy.egress, EgressPolicy::Deny);
        assert_eq!(reparsed.data_policy.inference, InferenceMode::LocalOnly);
        // In-memory apply and the single on-disk write agree.
        assert_eq!(reparsed.data_policy, candidate.data_policy);
    }

    #[test]
    fn inference_patch_apply_and_disk_write_agree() {
        let path = std::env::temp_dir().join(format!(
            "amatl-config-inference-patch-test-{}.toml",
            std::process::id()
        ));
        std::fs::write(
            &path,
            "schema_version = \"1\"\n\n\
             # A comment an operator wrote about something unrelated.\n\
             [inference]\n\
             remote_endpoint = \"https://old.example/embeddings\"\n\
             remote_timeout_ms = 5000\n",
        )
        .unwrap();

        let patch = InferenceConfigPatch {
            remote_endpoint: Some("https://api.deepinfra.com/v1/openai/embeddings".into()),
            remote_model: Some("BAAI/bge-base-en-v1.5".into()),
            embedding_dimensions: Some(384),
            ..Default::default()
        };

        // In-memory apply and the on-disk write must agree.
        let mut config = Config::from_toml(&std::fs::read_to_string(&path).unwrap()).unwrap();
        patch.apply(&mut config.inference);

        Config::set_inference_fields(&path, &patch).unwrap();
        let written = std::fs::read_to_string(&path).unwrap();
        assert!(written.contains("# A comment an operator wrote about something unrelated."));
        let reparsed = Config::from_toml(&written).unwrap();

        assert_eq!(
            reparsed.inference.remote_endpoint,
            config.inference.remote_endpoint
        );
        assert_eq!(
            reparsed.inference.remote_model,
            config.inference.remote_model
        );
        assert_eq!(
            reparsed.inference.embedding_dimensions,
            config.inference.embedding_dimensions
        );
        // Untouched by the patch, both in memory and on disk.
        assert_eq!(reparsed.inference.remote_timeout_ms, 5000);
        assert_eq!(config.inference.remote_timeout_ms, 5000);

        // An empty string clears a previously-set optional field.
        let clear = InferenceConfigPatch {
            remote_endpoint: Some(String::new()),
            ..Default::default()
        };
        clear.apply(&mut config.inference);
        assert_eq!(config.inference.remote_endpoint, None);
        Config::set_inference_fields(&path, &clear).unwrap();
        let reparsed = Config::from_toml(&std::fs::read_to_string(&path).unwrap()).unwrap();
        std::fs::remove_file(&path).ok();
        assert_eq!(reparsed.inference.remote_endpoint, None);
    }

    #[test]
    fn answer_patch_never_touches_enabled() {
        let path = std::env::temp_dir().join(format!(
            "amatl-config-answer-patch-test-{}.toml",
            std::process::id()
        ));
        std::fs::write(
            &path,
            "schema_version = \"1\"\n\n[answer]\nenabled = true\nmodel = \"old-model\"\n",
        )
        .unwrap();

        let patch = AnswerConfigPatch {
            model: Some("deepseek-ai/DeepSeek-V3".into()),
            max_sources: Some(4),
            ..Default::default()
        };
        Config::set_answer_fields(&path, &patch).unwrap();
        let written = std::fs::read_to_string(&path).unwrap();
        std::fs::remove_file(&path).ok();
        let reparsed = Config::from_toml(&written).unwrap();
        assert_eq!(
            reparsed.answer.model.as_deref(),
            Some("deepseek-ai/DeepSeek-V3")
        );
        assert_eq!(reparsed.answer.max_sources, 4);
        // `enabled` survives untouched.
        assert!(reparsed.answer.enabled);
    }

    #[test]
    fn upsert_provider_record_writes_only_the_named_provider() {
        let path = std::env::temp_dir().join(format!(
            "amatl-config-provider-record-test-{}.toml",
            std::process::id()
        ));
        std::fs::write(
            &path,
            "schema_version = \"1\"\n\n\
             # A comment an operator wrote about something unrelated.\n\
             [providers]\n\
             enabled = [\"searxng\"]\n\n\
             [providers.searxng]\n\
             adapter_version = \"searxng-v1\"\n",
        )
        .unwrap();

        let record = ProviderRuntimeConfig {
            adapter_version: Some("custom-v2".into()),
            approval_status: ApprovalStatus::Approved,
            reviewer: Some("Alexis Hernandez".into()),
            supported_regions: vec!["us".into(), "eu".into()],
            ..Default::default()
        };
        Config::upsert_provider_record(&path, "custom_archive", &record).unwrap();
        let written = std::fs::read_to_string(&path).unwrap();
        std::fs::remove_file(&path).ok();

        assert!(written.contains("# A comment an operator wrote about something unrelated."));
        let reparsed = Config::from_toml(&written).unwrap();
        // The existing provider and the `enabled` list are untouched.
        assert_eq!(reparsed.providers.enabled, vec!["searxng".to_string()]);
        assert_eq!(
            reparsed
                .providers
                .get("searxng")
                .unwrap()
                .adapter_version
                .as_deref(),
            Some("searxng-v1")
        );
        let custom = reparsed.providers.get("custom_archive").unwrap();
        assert_eq!(custom.adapter_version.as_deref(), Some("custom-v2"));
        assert_eq!(custom.approval_status, ApprovalStatus::Approved);
        assert_eq!(custom.reviewer.as_deref(), Some("Alexis Hernandez"));
        assert_eq!(
            custom.supported_regions,
            vec!["us".to_string(), "eu".to_string()]
        );
    }

    #[test]
    fn upsert_provider_record_rejects_an_invalid_name() {
        let path = std::env::temp_dir().join(format!(
            "amatl-config-provider-record-invalid-{}.toml",
            std::process::id()
        ));
        std::fs::write(&path, "schema_version = \"1\"\n").unwrap();
        let result =
            Config::upsert_provider_record(&path, "Not Valid!", &ProviderRuntimeConfig::default());
        std::fs::remove_file(&path).ok();
        assert!(result.is_err());
    }

    #[test]
    fn set_ranking_policy_replaces_the_section_and_keeps_other_sections_and_comments() {
        let path = std::env::temp_dir().join(format!(
            "amatl-config-ranking-policy-test-{}.toml",
            std::process::id()
        ));
        std::fs::write(
            &path,
            "schema_version = \"1\"\n\n\
             # A comment an operator wrote about something unrelated.\n\
             [ranking_policy]\n\
             version = \"v1\"\n\
             rrf_k = 60\n\n\
             [server]\n\
             port = 8080\n",
        )
        .unwrap();

        let policy = RankingPolicyV1 {
            rrf_k: 42,
            ..RankingPolicyV1::default()
        };
        Config::set_ranking_policy(&path, &policy).unwrap();
        let written = std::fs::read_to_string(&path).unwrap();
        std::fs::remove_file(&path).ok();

        assert!(written.contains("# A comment an operator wrote about something unrelated."));
        assert!(written.contains("port = 8080"));
        let reparsed = Config::from_toml(&written).unwrap();
        assert_eq!(reparsed.ranking_policy.rrf_k, 42);
        assert_eq!(reparsed.server.port, 8080);
    }

    #[test]
    fn set_ranking_v2_policy_writes_the_nested_table_creating_parents_as_needed() {
        let path = std::env::temp_dir().join(format!(
            "amatl-config-ranking-v2-policy-test-{}.toml",
            std::process::id()
        ));
        std::fs::write(&path, "schema_version = \"1\"\n").unwrap();

        let policy = RankingV2Policy::default();
        Config::set_ranking_v2_policy(&path, &policy).unwrap();
        let written = std::fs::read_to_string(&path).unwrap();
        assert!(written.contains("[deep.ranking_v2.policy]") || written.contains("[deep]"));
        let reparsed = Config::from_toml(&written).unwrap();
        assert_eq!(reparsed.deep.ranking_v2.policy, policy);

        // A second write replaces the same nested table, doesn't duplicate it.
        Config::set_gap_policy(&path, &GapPolicyV1::default()).unwrap();
        let written = std::fs::read_to_string(&path).unwrap();
        std::fs::remove_file(&path).ok();
        let reparsed = Config::from_toml(&written).unwrap();
        assert_eq!(reparsed.deep.ranking_v2.policy, policy);
        assert_eq!(reparsed.deep.gaps.policy, GapPolicyV1::default());
    }

    #[test]
    fn set_diversity_policy_alone_can_break_the_search_policy_cross_check() {
        // Documents the contract `set_diversity_policy`'s doc comment
        // promises: the caller must validate the *pair* before writing
        // either one, because `Config::validate` requires them to agree.
        let path = std::env::temp_dir().join(format!(
            "amatl-config-diversity-cross-check-test-{}.toml",
            std::process::id()
        ));
        std::fs::write(&path, "schema_version = \"1\"\n").unwrap();

        let mut diversity = DiversityPolicyV1::default();
        diversity.max_visible_per_domain += 1;
        Config::set_diversity_policy(&path, &diversity).unwrap();
        let written = std::fs::read_to_string(&path).unwrap();
        std::fs::remove_file(&path).ok();
        let reparsed = Config::from_toml(&written).unwrap();
        assert!(reparsed.validate().is_err());
    }

    #[test]
    fn persistence_patch_changes_only_the_requested_fields_and_keeps_comments() {
        let path = std::env::temp_dir().join(format!(
            "amatl-config-persistence-patch-test-{}.toml",
            std::process::id()
        ));
        std::fs::write(
            &path,
            "schema_version = \"1\"\n\n\
             # A comment an operator wrote about something unrelated.\n\
             [persistence]\n\
             enabled = true\n\
             path = \"amatl.sqlite3\"\n\
             history_retention_days = 90\n",
        )
        .unwrap();

        let patch = PersistenceConfigPatch {
            auto_backup_enabled: Some(true),
            auto_backup_interval_seconds: Some(7_200),
            ..Default::default()
        };
        let mut config = Config::from_toml(&std::fs::read_to_string(&path).unwrap()).unwrap();
        patch.apply(&mut config.persistence);

        Config::set_persistence_fields(&path, &patch).unwrap();
        let written = std::fs::read_to_string(&path).unwrap();
        std::fs::remove_file(&path).ok();

        assert!(written.contains("# A comment an operator wrote about something unrelated."));
        let reparsed = Config::from_toml(&written).unwrap();
        assert!(reparsed.persistence.auto_backup_enabled);
        assert_eq!(reparsed.persistence.auto_backup_interval_seconds, 7_200);
        // Untouched: not part of the patch, both in memory and on disk.
        assert!(reparsed.persistence.enabled);
        assert_eq!(reparsed.persistence.path, "amatl.sqlite3");
        assert_eq!(reparsed.persistence.history_retention_days, 90);
        assert_eq!(config.persistence.path, "amatl.sqlite3");
    }

    #[test]
    fn persistence_patch_backup_directory_empty_string_clears_it() {
        let path = std::env::temp_dir().join(format!(
            "amatl-config-persistence-clear-test-{}.toml",
            std::process::id()
        ));
        std::fs::write(
            &path,
            "schema_version = \"1\"\n\n[persistence]\nbackup_directory = \"/var/backups/amatl\"\n",
        )
        .unwrap();

        let clear = PersistenceConfigPatch {
            backup_directory: Some(String::new()),
            ..Default::default()
        };
        Config::set_persistence_fields(&path, &clear).unwrap();
        let written = std::fs::read_to_string(&path).unwrap();
        std::fs::remove_file(&path).ok();
        let reparsed = Config::from_toml(&written).unwrap();
        assert_eq!(reparsed.persistence.backup_directory, None);
    }

    #[test]
    fn telemetry_patch_writes_only_the_requested_fields() {
        let path = std::env::temp_dir().join(format!(
            "amatl-config-telemetry-patch-test-{}.toml",
            std::process::id()
        ));
        std::fs::write(&path, "schema_version = \"1\"\n").unwrap();

        let patch = TelemetryConfigPatch {
            retention_days: Some(45),
            ..Default::default()
        };
        Config::set_telemetry_fields(&path, &patch).unwrap();
        let written = std::fs::read_to_string(&path).unwrap();
        std::fs::remove_file(&path).ok();
        let reparsed = Config::from_toml(&written).unwrap();
        assert_eq!(reparsed.telemetry.retention_days, 45);
        assert!(!reparsed.telemetry.persistence_enabled);
    }

    #[test]
    fn deep_patch_writes_top_level_fields_without_touching_nested_tables() {
        let path = std::env::temp_dir().join(format!(
            "amatl-config-deep-patch-test-{}.toml",
            std::process::id()
        ));
        std::fs::write(
            &path,
            "schema_version = \"1\"\n\n\
             # A comment an operator wrote about something unrelated.\n\
             [deep]\n\
             max_fetches = 10\n\n\
             [deep.extractor]\n\
             executable = \"trafilatura\"\n\n\
             [deep.renderer]\n\
             enabled = false\n",
        )
        .unwrap();

        let patch = DeepConfigPatch {
            max_fetches: Some(20),
            timeout_ms: Some(30_000),
            ..Default::default()
        };
        Config::set_deep_fields(&path, &patch).unwrap();
        let written = std::fs::read_to_string(&path).unwrap();
        std::fs::remove_file(&path).ok();

        assert!(written.contains("# A comment an operator wrote about something unrelated."));
        let reparsed = Config::from_toml(&written).unwrap();
        assert_eq!(reparsed.deep.max_fetches, 20);
        assert_eq!(reparsed.deep.timeout_ms, 30_000);
        // Untouched nested tables, on disk and after reparsing.
        assert_eq!(reparsed.deep.extractor.executable, "trafilatura");
        assert!(!reparsed.deep.renderer.enabled);
        assert!(written.contains("[deep.extractor]"));
        assert!(written.contains("[deep.renderer]"));
    }

    #[test]
    fn deep_extractor_and_renderer_patches_write_into_their_own_nested_table() {
        let path = std::env::temp_dir().join(format!(
            "amatl-config-deep-nested-patch-test-{}.toml",
            std::process::id()
        ));
        std::fs::write(&path, "schema_version = \"1\"\n").unwrap();

        let extractor_patch = ExtractorConfigPatch {
            timeout_ms: Some(12_000),
            ..Default::default()
        };
        Config::set_deep_extractor_fields(&path, &extractor_patch).unwrap();

        let renderer_patch = RendererConfigPatch {
            enabled: Some(true),
            max_memory_mb: Some(1024),
            ..Default::default()
        };
        Config::set_deep_renderer_fields(&path, &renderer_patch).unwrap();

        let written = std::fs::read_to_string(&path).unwrap();
        std::fs::remove_file(&path).ok();
        let reparsed = Config::from_toml(&written).unwrap();
        assert_eq!(reparsed.deep.extractor.timeout_ms, 12_000);
        // Untouched: not part of the extractor patch.
        assert_eq!(reparsed.deep.extractor.executable, "trafilatura");
        assert!(reparsed.deep.renderer.enabled);
        assert_eq!(reparsed.deep.renderer.max_memory_mb, 1024);
    }

    #[test]
    fn renderer_patch_enabling_under_isolated_profile_fails_validate_before_any_write() {
        // Documents the contract `RendererConfigPatch`'s doc comment
        // promises: the caller validates a candidate first, and this
        // combination is exactly what `Config::validate` rejects.
        let mut config = Config::default();
        config.data_policy.profile = SecurityProfile::Isolated;
        config.data_policy.egress = EgressPolicy::Deny;
        let patch = RendererConfigPatch {
            enabled: Some(true),
            ..Default::default()
        };
        patch.apply(&mut config.deep.renderer);
        assert!(config.validate().is_err());
    }

    #[test]
    fn server_patch_writes_only_the_requested_fields_and_keeps_comments() {
        let path = std::env::temp_dir().join(format!(
            "amatl-config-server-patch-test-{}.toml",
            std::process::id()
        ));
        std::fs::write(
            &path,
            "schema_version = \"1\"\n\n\
             # A comment an operator wrote about something unrelated.\n\
             [server]\n\
             port = 8080\n\
             rate_limit_per_minute = 60\n",
        )
        .unwrap();

        let patch = ServerConfigPatch {
            allowed_hosts: Some(vec!["example.internal".into()]),
            request_timeout_ms: Some(45_000),
            ..Default::default()
        };
        Config::set_server_fields(&path, &patch).unwrap();
        let written = std::fs::read_to_string(&path).unwrap();
        std::fs::remove_file(&path).ok();

        assert!(written.contains("# A comment an operator wrote about something unrelated."));
        let reparsed = Config::from_toml(&written).unwrap();
        assert_eq!(
            reparsed.server.allowed_hosts,
            vec!["example.internal".to_string()]
        );
        assert_eq!(reparsed.server.request_timeout_ms, 45_000);
        // Untouched: not part of the patch.
        assert_eq!(reparsed.server.port, 8080);
        assert_eq!(reparsed.server.rate_limit_per_minute, 60);
    }

    #[test]
    fn server_patch_tls_fields_write_the_nested_table_and_clear_via_empty_string() {
        let path = std::env::temp_dir().join(format!(
            "amatl-config-server-tls-patch-test-{}.toml",
            std::process::id()
        ));
        std::fs::write(&path, "schema_version = \"1\"\n").unwrap();

        let set = ServerConfigPatch {
            tls_cert_path: Some("/etc/amatl/cert.pem".into()),
            tls_key_path: Some("/etc/amatl/key.pem".into()),
            ..Default::default()
        };
        Config::set_server_fields(&path, &set).unwrap();
        let reparsed = Config::from_toml(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            reparsed.server.tls.cert_path.as_deref(),
            Some("/etc/amatl/cert.pem")
        );

        let clear = ServerConfigPatch {
            tls_cert_path: Some(String::new()),
            tls_key_path: Some(String::new()),
            ..Default::default()
        };
        Config::set_server_fields(&path, &clear).unwrap();
        let written = std::fs::read_to_string(&path).unwrap();
        std::fs::remove_file(&path).ok();
        let reparsed = Config::from_toml(&written).unwrap();
        assert_eq!(reparsed.server.tls.cert_path, None);
        assert_eq!(reparsed.server.tls.key_path, None);
    }

    #[test]
    fn server_patch_changed_fields_classifies_hot_and_cold_correctly() {
        let patch = ServerConfigPatch {
            port: Some(9090),
            allowed_hosts: Some(vec!["example.internal".into()]),
            tls_cert_path: Some("/etc/amatl/cert.pem".into()),
            ..Default::default()
        };
        let changed = patch.changed_fields();
        assert_eq!(changed.len(), 3);
        assert!(changed.contains(&("server", "port")));
        assert!(changed.contains(&("server", "allowed_hosts")));
        assert!(changed.contains(&("server.tls", "cert_path")));

        let classified: Vec<ReloadKind> = changed
            .iter()
            .map(|(section, key)| ReloadKind::of(section, key))
            .collect();
        assert!(classified.contains(&ReloadKind::Cold)); // port, tls.cert_path
        assert!(classified.contains(&ReloadKind::Hot)); // allowed_hosts
        assert_eq!(ReloadKind::of("server", "port"), ReloadKind::Cold);
        assert_eq!(ReloadKind::of("server", "allowed_hosts"), ReloadKind::Hot);
        assert_eq!(ReloadKind::of("server.tls", "cert_path"), ReloadKind::Cold);
    }
}
