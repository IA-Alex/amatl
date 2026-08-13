use crate::diversity::DiversityPolicyV1;
use crate::gaps::GapPolicyV1;
use crate::progressive::SearchPolicyV1;
use crate::ranking::RankingPolicyV1;
use crate::ranking_v2::RankingV2Policy;
use serde::{Deserialize, Serialize};
use std::path::Path;
use thiserror::Error;

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Config {
    pub data_policy: DataPolicyConfig,
    pub providers: ProviderConfig,
    pub timeouts: TimeoutConfig,
    pub budget: BudgetConfig,
    pub execution: ExecutionConfig,
    pub ranking_policy: RankingPolicyV1,
    pub diversity_policy: DiversityPolicyV1,
    pub search_policy: SearchPolicyV1,
    pub persistence: PersistenceConfig,
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

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ServerConfig {
    pub bind: String,
    pub port: u16,
    pub token_env: String,
    pub no_auth: bool,
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

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct TlsConfig {
    pub cert_path: Option<String>,
    pub key_path: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ProviderConfig {
    pub enabled: Vec<String>,
    pub brave: ProviderRuntimeConfig,
    pub mojeek: ProviderRuntimeConfig,
    pub duckduckgo_html: ProviderRuntimeConfig,
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
        Self {
            enabled: vec![],
            brave,
            mojeek,
            duckduckgo_html: ProviderRuntimeConfig::default(),
        }
    }
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
            version: "trafilatura-cli-v1".into(),
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
        const KNOWN_PROVIDERS: [&str; 3] = ["brave", "mojeek", "duckduckgo_html"];
        if let Some(name) = self
            .providers
            .enabled
            .iter()
            .find(|name| !KNOWN_PROVIDERS.contains(&name.as_str()))
        {
            return Err(ConfigError::Policy(format!(
                "unknown enabled provider: {name}"
            )));
        }
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
            || self.telemetry.retention_days == 0
            || self.cache.document.ttl_seconds == 0
            || self.cache.document.max_entries == 0
            || self.cache.document.max_bytes == 0
        {
            return Err(ConfigError::Policy(
                "cache and telemetry limits must be positive".into(),
            ));
        }
        if self.telemetry.retention_days != 30 {
            return Err(ConfigError::Policy(
                "telemetry v1 retention must remain 30 days".into(),
            ));
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
        assert!(!config.providers.brave.approved());
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
}
