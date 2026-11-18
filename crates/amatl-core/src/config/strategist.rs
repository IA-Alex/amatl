use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct StrategistConfig {
    pub enabled: bool,
    pub endpoint: Option<String>,
    pub model: Option<String>,
    pub credential_env: Option<String>,
    pub timeout_ms: u64,
}

impl Default for StrategistConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            endpoint: None,
            model: None,
            credential_env: None,
            timeout_ms: 800,
        }
    }
}

/// Validation for the `[strategist]` section, kept alongside the type.
///
/// The strategist is a distinct LLM capability (a chat-completions call that
/// proposes a refined query), independent of the ranking-embeddings mode that
/// `[data_policy].inference` governs. So unlike `validate_answer` it does not
/// require `inference = "remote_explicit"`: it only needs network egress
/// (standard profile, governed egress) plus a usable endpoint, model and a
/// named credential environment variable. While `enabled` is false the
/// section is not forced to be complete; once `enabled` is true it must be
/// real — a missing bit fails closed rather than silently enabling a no-op.
impl super::Config {
    pub(super) fn validate_strategist(&self) -> Result<(), super::ConfigError> {
        if !(1..=60_000).contains(&self.strategist.timeout_ms) {
            return Err(super::ConfigError::Policy(
                "invalid strategist limit".into(),
            ));
        }
        if !self.strategist.enabled {
            return Ok(());
        }
        if !self.data_policy.allows_network_egress() {
            return Err(super::ConfigError::Policy(
                "strategist.enabled requires network egress (standard profile, governed egress)"
                    .into(),
            ));
        }
        let endpoint = self.strategist.endpoint.as_deref().ok_or_else(|| {
            super::ConfigError::Policy("strategist.enabled requires strategist.endpoint".into())
        })?;
        crate::inference::validate_remote_endpoint(endpoint)
            .map_err(|error| super::ConfigError::Policy(error.to_string()))?;
        if self.strategist.model.as_deref().is_none_or(str::is_empty) {
            return Err(super::ConfigError::Policy(
                "strategist.enabled requires strategist.model".into(),
            ));
        }
        if self
            .strategist
            .credential_env
            .as_deref()
            .is_none_or(str::is_empty)
        {
            return Err(super::ConfigError::Policy(
                "strategist.enabled requires strategist.credential_env".into(),
            ));
        }
        Ok(())
    }
}
