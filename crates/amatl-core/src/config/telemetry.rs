use super::*;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct TelemetryConfig {
    pub persistence_enabled: bool,
    pub retention_days: u32,
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

impl Config {
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

    /// Telemetry retention window.
    pub(super) fn validate_telemetry(&self) -> Result<(), ConfigError> {
        if self.telemetry.retention_days < crate::telemetry::TELEMETRY_MIN_RETENTION_DAYS
            || self.telemetry.retention_days > crate::telemetry::TELEMETRY_MAX_RETENTION_DAYS
        {
            return Err(ConfigError::Policy(format!(
                "telemetry retention_days must be between {} and {}",
                crate::telemetry::TELEMETRY_MIN_RETENTION_DAYS,
                crate::telemetry::TELEMETRY_MAX_RETENTION_DAYS
            )));
        }
        Ok(())
    }
}
