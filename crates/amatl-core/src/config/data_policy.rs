use super::*;
use serde::{Deserialize, Serialize};
use std::path::Path;

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
impl Default for DataPolicyConfig {
    fn default() -> Self {
        Self {
            profile: SecurityProfile::Standard,
            egress: EgressPolicy::Governed,
            inference: InferenceMode::Disabled,
        }
    }
}

impl Config {
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
}
