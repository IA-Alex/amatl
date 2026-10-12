use super::*;
use serde::{Deserialize, Serialize};
use std::path::Path;

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

impl Config {
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
        write_config_document(path, &document)?;
        Ok(())
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
