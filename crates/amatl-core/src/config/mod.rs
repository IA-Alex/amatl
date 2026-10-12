use crate::diversity::DiversityPolicyV1;
use crate::gaps::GapPolicyV1;
use crate::progressive::SearchPolicyV1;
use crate::ranking::RankingPolicyV1;
use crate::ranking_v2::RankingV2Policy;
use serde::{Deserialize, Serialize};
use std::path::Path;
use thiserror::Error;
use time::Date;

mod data_policy;
mod deep_config;
mod execution;
mod inference;
mod persistence;
mod providers;
mod ranking;
mod server;
mod strategist;
mod telemetry;

pub use data_policy::*;
pub use deep_config::*;
pub use execution::*;
pub use inference::*;
pub use persistence::*;
pub use providers::*;
pub use ranking::*;
pub use server::*;
pub use strategist::*;
pub use telemetry::*;

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
    pub strategist: StrategistConfig,
    pub providers: ProviderConfig,
    pub timeouts: TimeoutConfig,
    pub budget: BudgetConfig,
    pub execution: ExecutionConfig,
    pub ranking_policy: RankingPolicyV1,
    /// Post-rank adjustments outside the protected `RankingPolicyV1` formula
    /// (`[ranking.optics]` today).
    pub ranking: RankingConfig,
    pub diversity_policy: DiversityPolicyV1,
    pub search_policy: SearchPolicyV1,
    /// Primary / expansion provider roles (STEP 1). When
    /// [`ExpansionConfig::mode`] is `Legacy` (the default) this section is
    /// inert and routing keeps its pre-role behavior; when
    /// `PrimaryExpansion` the primary provider always occupies the first
    /// round and the expansion providers are only reached in later rounds.
    pub expansion: ExpansionConfig,
    pub persistence: PersistenceConfig,
    /// Trip limits for the persistent provider circuit breaker.
    pub circuit_breaker: crate::circuit::CircuitPolicy,
    pub cache: CacheConfig,
    pub telemetry: TelemetryConfig,
    pub deep: DeepConfig,
    pub server: ServerConfig,
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

fn epoch_days() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| (duration.as_secs() / 86_400) as i64)
        .unwrap_or(i64::MAX)
}

fn current_utc_day() -> i64 {
    let epoch =
        time::Date::from_calendar_date(1970, time::Month::January, 1).unwrap_or(time::Date::MIN);
    epoch_days().saturating_add(epoch.to_julian_day() as i64)
}

fn day_from_iso_date(value: &str) -> Option<i64> {
    Date::parse(value, &time::format_description::well_known::Iso8601::DATE)
        .ok()
        .map(|date| date.to_julian_day() as i64)
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
            ranking: RankingConfig::default(),
            diversity_policy: DiversityPolicyV1::default(),
            search_policy: SearchPolicyV1::default(),
            expansion: ExpansionConfig::default(),
            persistence: PersistenceConfig::default(),
            circuit_breaker: crate::circuit::CircuitPolicy::default(),
            cache: CacheConfig::default(),
            telemetry: TelemetryConfig::default(),
            strategist: StrategistConfig::default(),
            deep: DeepConfig::default(),
            server: ServerConfig::default(),
        }
    }
}

/// Render and write a configuration document atomically.
///
/// Writes to a sibling temp file, fsyncs, then renames over the target. A
/// reader — including a concurrent `POST /reload` — therefore never observes a
/// truncated file, and a crash mid-write cannot corrupt the only copy the way a
/// truncating `std::fs::write` can.
pub(crate) fn write_config_document(
    path: &Path,
    document: &toml_edit::DocumentMut,
) -> Result<(), ConfigError> {
    use std::io::Write;
    let temp = path.with_extension(format!("tmp.{}", std::process::id()));
    let mut file = std::fs::File::create(&temp)?;
    file.write_all(document.to_string().as_bytes())?;
    file.sync_all()?;
    std::fs::rename(&temp, path)?;
    Ok(())
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
        write_config_document(path, &document)?;
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
        write_config_document(path, &document)?;
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
        write_config_document(path, &document)?;
        Ok(())
    }

    /// Thin orchestrator: validates the two things that belong to no single
    /// section (schema version, provider-name shape), delegates every section's
    /// own rules to its `validate_<section>` method, and then runs the handful
    /// of checks that genuinely span two or more sections and therefore cannot
    /// live inside any one of them.
    ///
    /// The delegation order preserves the historical order in which a bad
    /// config failed, so error messages a caller may have pinned do not move.
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
        self.validate_strategist()?;
        self.validate_execution()?;
        self.validate_persistence()?;
        self.validate_ranking_policies()?;
        self.validate_expansion()?;
        self.validate_cache()?;
        self.validate_telemetry()?;

        // ── Cross-section checks ────────────────────────────────────────────
        // These stay here because each one reads fields from two or more
        // sections at once; moving one into a `validate_<section>` would make
        // that method reach across into a sibling section's state, which is
        // exactly the coupling the per-section split is meant to remove.

        // persistence ⇄ cache ⇄ telemetry: the SQLite-backed features cannot
        // run without SQLite persistence itself being on.
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

        self.validate_deep()?;

        // `server.bind` is parsed once here and handed to `validate_server`,
        // because the isolated-profile check below also needs it and parsing
        // it twice would duplicate the "must be an IP address" error site.
        let bind = self
            .server
            .bind
            .parse::<std::net::IpAddr>()
            .map_err(|_| ConfigError::Policy("server bind must be an IP address".into()))?;

        // data_policy ⇄ server ⇄ deep ⇄ inference: the isolated profile
        // constrains egress, remote inference, the server bind and the
        // renderer all at once.
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

        // data_policy ⇄ providers, data_policy ⇄ inference: denied egress
        // forbids both enabled providers and remote inference.
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

        self.validate_server(bind)?;
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

fn is_provider_key(name: &str) -> bool {
    !name.is_empty()
        && name.chars().all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '_'
        })
}

fn default_schema_version() -> String {
    crate::SCHEMA_VERSION.to_string()
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
    fn expansion_defaults_are_legacy_and_valid() {
        let config = Config::default();
        assert_eq!(config.expansion.mode, ExpansionMode::Legacy);
        assert!(!config.expansion.role_model_active());
        assert_eq!(config.expansion.primary_provider, "marginalia");
        assert_eq!(config.expansion.expansion_providers, ["searxng"]);
        assert!(config.validate().is_ok());
    }

    #[test]
    fn expansion_primary_expansion_mode_validates_declared_roles() {
        let mut config = Config::default();
        config.expansion.mode = ExpansionMode::PrimaryExpansion;
        assert!(config.validate().is_ok());

        config.expansion.primary_provider = "not_a_provider".into();
        assert!(config.validate().is_err());
    }

    #[test]
    fn expansion_rejects_provider_in_both_roles_and_empty_expansion() {
        let mut both = Config::default();
        both.expansion.mode = ExpansionMode::PrimaryExpansion;
        both.expansion.expansion_providers = vec!["marginalia".into()];
        assert!(both.validate().is_err());

        let mut empty = Config::default();
        empty.expansion.mode = ExpansionMode::PrimaryExpansion;
        empty.expansion.expansion_providers = vec![];
        assert!(empty.validate().is_err());

        let mut repeated = Config::default();
        repeated.expansion.mode = ExpansionMode::PrimaryExpansion;
        repeated.expansion.expansion_providers = vec!["searxng".into(), "searxng".into()];
        assert!(repeated.validate().is_err());
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
        inference_batch.inference.local_cache_capacity = usize::MAX;
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
        //
        // `validate()` checks `Path::is_absolute()`, which is platform-specific:
        // `/opt/amatl/vectors.txt` is absolute on Unix but not on Windows (no
        // drive letter / UNC prefix), so the accepted path must be chosen per
        // platform to keep this test portable.
        #[cfg(windows)]
        let absolute_model_path = r#"local_model_path = 'C:\amatl\vectors.txt'"#;
        #[cfg(not(windows))]
        let absolute_model_path = r#"local_model_path = "/opt/amatl/vectors.txt""#;
        let accepted = Config::from_toml(&format!(
            r#"
            [inference]
            backend = "local_model_v1"
            {absolute_model_path}
            "#,
        ))
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

    /// Configs written against rc.1 spell the field `local_model_batch`; the
    /// `#[serde(alias)]` keeps them loading with the same effect after the
    /// rename to `local_cache_capacity`.
    // TODO(1.0): drop with the alias.
    #[test]
    fn pre_rc_local_model_batch_key_is_still_accepted_as_an_alias() {
        let config = Config::from_toml("[inference]\nlocal_model_batch = 7\n").unwrap();
        assert_eq!(config.inference.local_cache_capacity, 7);
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
    fn set_strategist_enabled_flips_only_that_key_and_keeps_comments() {
        let path = std::env::temp_dir().join(format!(
            "amatl-config-strategist-toggle-test-{}.toml",
            std::process::id()
        ));
        std::fs::write(
            &path,
            "schema_version = \"1\"\n\n\
             # A comment an operator wrote about something unrelated.\n\
             [server]\n\
             port = 8080\n\n\
             [strategist]\n\
             enabled = false\n\
             model = \"deepseek-ai/DeepSeek-V3\"\n",
        )
        .unwrap();

        Config::set_strategist_enabled(&path, true).unwrap();
        let written = std::fs::read_to_string(&path).unwrap();
        std::fs::remove_file(&path).ok();

        assert!(written.contains("enabled = true"));
        assert!(written.contains("# A comment an operator wrote about something unrelated."));
        assert!(written.contains("model = \"deepseek-ai/DeepSeek-V3\""));
        assert!(written.contains("port = 8080"));
        let reparsed = Config::from_toml(&written).unwrap();
        assert!(reparsed.strategist.enabled);
        assert_eq!(reparsed.server.port, 8080);
    }

    #[test]
    fn strategist_enabled_requires_endpoint_model_credential_and_egress() {
        // A default (disabled) strategist validates fine even incomplete.
        let mut config = Config::default();
        config.strategist.enabled = true;
        config.strategist.endpoint = None;
        // No endpoint -> must fail closed, not silently accept a no-op enable.
        assert!(config.validate().is_err(), "missing endpoint must fail");

        // With endpoint + model but no credential -> must fail.
        let mut config = Config::default();
        config.strategist.enabled = true;
        config.strategist.endpoint =
            Some("https://api.deepinfra.com/v1/openai/chat/completions".into());
        config.strategist.model = Some("deepseek-ai/DeepSeek-V3".into());
        assert!(
            config.validate().is_err(),
            "enabled without credential must fail"
        );

        // Fully configured with network egress -> validates, and does NOT need
        // remote_explicit (the strategist is a chat call independent of the
        // ranking-embeddings mode, so `inference` can stay disabled).
        let mut config = Config::default();
        config.strategist.enabled = true;
        config.strategist.endpoint =
            Some("https://api.deepinfra.com/v1/openai/chat/completions".into());
        config.strategist.model = Some("deepseek-ai/DeepSeek-V3".into());
        config.strategist.credential_env = Some("AMATL_TEST_KEY".into());
        assert_eq!(
            config.data_policy.inference,
            crate::config::InferenceMode::Disabled
        );
        assert!(config.validate().is_ok(), "{:?}", config.validate().err());
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

    /// Distinct temp path per (tag, PID, call); the nanosecond suffix keeps
    /// two parallel tests sharing a `tag` in one PID from colliding.
    fn unique_tmp_path(tag: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "amatl-mirror-{tag}-{}-{}.toml",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    // Mirror tests: for each patchable config section, build a patch with
    // *every* field set to a value that differs from that field's real
    // Default, apply it via `set_<section>_fields`, reload with `from_toml`,
    // and assert each field round-tripped. A field the patch struct still
    // declares but `set_<section>_fields` silently drops is caught here; the
    // compiler already catches the inverse (a field the writer sets but the
    // patch does not declare).

    #[test]
    fn mirror_round_trips_every_data_policy_patch_field() {
        let path = unique_tmp_path("data-policy");
        std::fs::write(&path, "schema_version = \"1\"\n").unwrap();

        let patch = DataPolicyConfigPatch {
            profile: Some(SecurityProfile::Isolated),
            egress: Some(EgressPolicy::Deny),
            inference: Some(InferenceMode::LocalOnly),
        };
        Config::set_data_policy_fields(&path, &patch).unwrap();

        let reloaded = Config::from_toml(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(reloaded.data_policy.profile, SecurityProfile::Isolated);
        assert_eq!(reloaded.data_policy.egress, EgressPolicy::Deny);
        assert_eq!(reloaded.data_policy.inference, InferenceMode::LocalOnly);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn mirror_round_trips_every_inference_patch_field() {
        let path = unique_tmp_path("inference");
        std::fs::write(&path, "schema_version = \"1\"\n").unwrap();

        let patch = InferenceConfigPatch {
            backend: Some("local_model_v1".into()),
            embedding_dimensions: Some(128),
            max_documents: Some(32),
            max_input_chars: Some(5_000),
            reranker_prior_weight: Some(0.25),
            local_model_path: Some("/models/vectors.txt".into()),
            local_cache_capacity: Some(16),
            local_cache_path: Some("/cache.json".into()),
            remote_endpoint: Some("https://e.test/v1".into()),
            remote_model: Some("m".into()),
            remote_credential_env: Some("ENV".into()),
            remote_timeout_ms: Some(3_000),
            remote_max_batch: Some(8),
        };
        Config::set_inference_fields(&path, &patch).unwrap();

        let reloaded = Config::from_toml(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(reloaded.inference.backend, "local_model_v1");
        assert_eq!(reloaded.inference.embedding_dimensions, 128);
        assert_eq!(reloaded.inference.max_documents, 32);
        assert_eq!(reloaded.inference.max_input_chars, 5_000);
        assert_eq!(reloaded.inference.reranker_prior_weight, 0.25);
        assert_eq!(
            reloaded.inference.local_model_path.as_deref(),
            Some("/models/vectors.txt")
        );
        assert_eq!(reloaded.inference.local_cache_capacity, 16);
        assert_eq!(
            reloaded.inference.local_cache_path.as_deref(),
            Some("/cache.json")
        );
        assert_eq!(
            reloaded.inference.remote_endpoint.as_deref(),
            Some("https://e.test/v1")
        );
        assert_eq!(reloaded.inference.remote_model.as_deref(), Some("m"));
        assert_eq!(
            reloaded.inference.remote_credential_env.as_deref(),
            Some("ENV")
        );
        assert_eq!(reloaded.inference.remote_timeout_ms, 3_000);
        assert_eq!(reloaded.inference.remote_max_batch, 8);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn mirror_round_trips_every_answer_patch_field() {
        let path = unique_tmp_path("answer");
        std::fs::write(&path, "schema_version = \"1\"\n").unwrap();

        let patch = AnswerConfigPatch {
            endpoint: Some("https://answer.test/v1".into()),
            model: Some("answer-model".into()),
            credential_env: Some("ANSWER_ENV".into()),
            timeout_ms: Some(15_000),
            max_sources: Some(4),
            max_source_chars: Some(800),
            max_answer_tokens: Some(500),
        };
        Config::set_answer_fields(&path, &patch).unwrap();

        let reloaded = Config::from_toml(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            reloaded.answer.endpoint.as_deref(),
            Some("https://answer.test/v1")
        );
        assert_eq!(reloaded.answer.model.as_deref(), Some("answer-model"));
        assert_eq!(
            reloaded.answer.credential_env.as_deref(),
            Some("ANSWER_ENV")
        );
        assert_eq!(reloaded.answer.timeout_ms, 15_000);
        assert_eq!(reloaded.answer.max_sources, 4);
        assert_eq!(reloaded.answer.max_source_chars, 800);
        assert_eq!(reloaded.answer.max_answer_tokens, 500);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn mirror_round_trips_every_persistence_patch_field() {
        let path = unique_tmp_path("persistence");
        std::fs::write(&path, "schema_version = \"1\"\n").unwrap();

        let patch = PersistenceConfigPatch {
            history_enabled: Some(false),
            saved_document_max_bytes: Some(2_097_152),
            saved_documents_retention_days: Some(365),
            audit_retention_days: Some(30),
            history_retention_days: Some(14),
            cache_retention_days: Some(14),
            document_cache_retention_days: Some(14),
            purge_interval_seconds: Some(1_800),
            auto_backup_enabled: Some(true),
            auto_backup_interval_seconds: Some(7_200),
            auto_backup_max_count: Some(5),
            backup_directory: Some("/backups".into()),
        };
        Config::set_persistence_fields(&path, &patch).unwrap();

        let reloaded = Config::from_toml(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert!(!reloaded.persistence.history_enabled);
        assert_eq!(reloaded.persistence.saved_document_max_bytes, 2_097_152);
        assert_eq!(reloaded.persistence.saved_documents_retention_days, 365);
        assert_eq!(reloaded.persistence.audit_retention_days, 30);
        assert_eq!(reloaded.persistence.history_retention_days, 14);
        assert_eq!(reloaded.persistence.cache_retention_days, 14);
        assert_eq!(reloaded.persistence.document_cache_retention_days, 14);
        assert_eq!(reloaded.persistence.purge_interval_seconds, 1_800);
        assert!(reloaded.persistence.auto_backup_enabled);
        assert_eq!(reloaded.persistence.auto_backup_interval_seconds, 7_200);
        assert_eq!(reloaded.persistence.auto_backup_max_count, 5);
        assert_eq!(
            reloaded.persistence.backup_directory.as_deref(),
            Some("/backups")
        );

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn mirror_round_trips_every_telemetry_patch_field() {
        let path = unique_tmp_path("telemetry");
        std::fs::write(&path, "schema_version = \"1\"\n").unwrap();

        let patch = TelemetryConfigPatch {
            persistence_enabled: Some(true),
            retention_days: Some(90),
        };
        Config::set_telemetry_fields(&path, &patch).unwrap();

        let reloaded = Config::from_toml(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert!(reloaded.telemetry.persistence_enabled);
        assert_eq!(reloaded.telemetry.retention_days, 90);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn mirror_round_trips_every_deep_patch_field() {
        let path = unique_tmp_path("deep");
        std::fs::write(&path, "schema_version = \"1\"\n").unwrap();

        let patch = DeepConfigPatch {
            top_k: Some(3),
            max_fetches: Some(5),
            max_bytes: Some(10_485_760),
            max_redirects: Some(3),
            max_crawl_urls: Some(5),
            max_depth: Some(2),
            respect_robots: Some(false),
            robots_timeout_ms: Some(2_000),
            robots_max_bytes: Some(262_144),
            timeout_ms: Some(10_000),
        };
        Config::set_deep_fields(&path, &patch).unwrap();

        let reloaded = Config::from_toml(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(reloaded.deep.top_k, 3);
        assert_eq!(reloaded.deep.max_fetches, 5);
        assert_eq!(reloaded.deep.max_bytes, 10_485_760);
        assert_eq!(reloaded.deep.max_redirects, 3);
        assert_eq!(reloaded.deep.max_crawl_urls, 5);
        assert_eq!(reloaded.deep.max_depth, 2);
        assert!(!reloaded.deep.respect_robots);
        assert_eq!(reloaded.deep.robots_timeout_ms, 2_000);
        assert_eq!(reloaded.deep.robots_max_bytes, 262_144);
        assert_eq!(reloaded.deep.timeout_ms, 10_000);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn mirror_round_trips_every_deep_extractor_patch_field() {
        let path = unique_tmp_path("deep-extractor");
        std::fs::write(&path, "schema_version = \"1\"\n").unwrap();

        let patch = ExtractorConfigPatch {
            executable: Some("custom-trafilatura".into()),
            version: Some("v2".into()),
            timeout_ms: Some(4_000),
            max_output_bytes: Some(2_097_152),
        };
        Config::set_deep_extractor_fields(&path, &patch).unwrap();

        let reloaded = Config::from_toml(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(reloaded.deep.extractor.executable, "custom-trafilatura");
        assert_eq!(reloaded.deep.extractor.version, "v2");
        assert_eq!(reloaded.deep.extractor.timeout_ms, 4_000);
        assert_eq!(reloaded.deep.extractor.max_output_bytes, 2_097_152);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn mirror_round_trips_every_deep_renderer_patch_field() {
        let path = unique_tmp_path("deep-renderer");
        std::fs::write(&path, "schema_version = \"1\"\n").unwrap();

        let patch = RendererConfigPatch {
            enabled: Some(true),
            max_browser_calls: Some(1),
            timeout_ms: Some(4_000),
            shutdown_grace_ms: Some(300),
            max_memory_mb: Some(256),
            max_redirects: Some(3),
            sandbox_path: Some("custom-sandbox".into()),
            max_dom_bytes: Some(4_194_304),
        };
        Config::set_deep_renderer_fields(&path, &patch).unwrap();

        let reloaded = Config::from_toml(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert!(reloaded.deep.renderer.enabled);
        assert_eq!(reloaded.deep.renderer.max_browser_calls, 1);
        assert_eq!(reloaded.deep.renderer.timeout_ms, 4_000);
        assert_eq!(reloaded.deep.renderer.shutdown_grace_ms, 300);
        assert_eq!(reloaded.deep.renderer.max_memory_mb, 256);
        assert_eq!(reloaded.deep.renderer.max_redirects, 3);
        assert_eq!(reloaded.deep.renderer.sandbox_path, "custom-sandbox");
        assert_eq!(reloaded.deep.renderer.max_dom_bytes, 4_194_304);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn mirror_round_trips_every_ranking_optics_patch_field() {
        let path = unique_tmp_path("ranking-optics");
        std::fs::write(&path, "schema_version = \"1\"\n").unwrap();

        let patch = OpticsConfigPatch {
            enabled: Some(true),
            path: Some("amatl.optic".into()),
        };
        Config::set_ranking_optics_fields(&path, &patch).unwrap();

        let written = std::fs::read_to_string(&path).unwrap();
        assert!(written.contains("[ranking.optics]") || written.contains("[ranking]"));

        let reloaded = Config::from_toml(&written).unwrap();
        assert!(reloaded.ranking.optics.enabled);
        assert_eq!(reloaded.ranking.optics.path.as_deref(), Some("amatl.optic"));

        // The in-memory patch mirrors the file writer exactly.
        let mut in_memory = OpticsConfig::default();
        patch.apply(&mut in_memory);
        assert_eq!(in_memory, reloaded.ranking.optics);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn ranking_optics_defaults_to_disabled_and_no_path() {
        let config = Config::default();
        assert!(!config.ranking.optics.enabled);
        assert!(config.ranking.optics.path.is_none());
    }

    #[test]
    fn mirror_round_trips_every_server_patch_field() {
        let path = unique_tmp_path("server");
        std::fs::write(&path, "schema_version = \"1\"\n").unwrap();

        let patch = ServerConfigPatch {
            bind: Some("0.0.0.0".into()),
            port: Some(9_090),
            token_env: Some("CUSTOM_TOKEN".into()),
            no_auth: Some(true),
            allowed_hosts: Some(vec!["example.com".into()]),
            allowed_origins: Some(vec!["https://example.com".into()]),
            max_body_bytes: Some(32_768),
            max_header_bytes: Some(8_192),
            request_timeout_ms: Some(15_000),
            idle_timeout_ms: Some(15_000),
            rate_limit_per_minute: Some(30),
            max_connections: Some(32),
            tls_cert_path: Some("/tls/cert.pem".into()),
            tls_key_path: Some("/tls/key.pem".into()),
        };
        Config::set_server_fields(&path, &patch).unwrap();

        let reloaded = Config::from_toml(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(reloaded.server.bind, "0.0.0.0");
        assert_eq!(reloaded.server.port, 9_090);
        assert_eq!(reloaded.server.token_env, "CUSTOM_TOKEN");
        assert!(reloaded.server.no_auth);
        assert_eq!(
            reloaded.server.allowed_hosts,
            vec!["example.com".to_string()]
        );
        assert_eq!(
            reloaded.server.allowed_origins,
            vec!["https://example.com".to_string()]
        );
        assert_eq!(reloaded.server.max_body_bytes, 32_768);
        assert_eq!(reloaded.server.max_header_bytes, 8_192);
        assert_eq!(reloaded.server.request_timeout_ms, 15_000);
        assert_eq!(reloaded.server.idle_timeout_ms, 15_000);
        assert_eq!(reloaded.server.rate_limit_per_minute, 30);
        assert_eq!(reloaded.server.max_connections, 32);
        assert_eq!(
            reloaded.server.tls.cert_path.as_deref(),
            Some("/tls/cert.pem")
        );
        assert_eq!(
            reloaded.server.tls.key_path.as_deref(),
            Some("/tls/key.pem")
        );

        let _ = std::fs::remove_file(&path);
    }
}
