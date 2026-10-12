use super::*;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct PersistenceConfig {
    pub enabled: bool,
    pub path: String,
    /// Record every executed search in the local history table. Requires
    /// `enabled`; history never leaves the machine.
    pub history_enabled: bool,
    /// Upper bound on the payload a surface may persist as a saved document.
    /// The transport must be able to carry it: `server.max_body_bytes` (default
    /// 64 KiB) must be at least this value plus JSON overhead, or `POST /saved`
    /// returns `413 body_too_large` before the service ever sees the payload.
    pub saved_document_max_bytes: u64,
    /// Days saved documents are kept (0 = unlimited). Requires `enabled`.
    /// Defaults to unlimited because saved documents are explicit user data.
    pub saved_documents_retention_days: u32,
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
    /// Reserved: when non-zero, stale entries within this window would be
    /// served while a background revalidation is triggered. No production read
    /// path calls the stale-while-revalidate accessor today — Deep revalidates
    /// with conditional headers (`If-None-Match` / `If-Modified-Since`) — so
    /// this value currently has no effect. Kept so existing configurations
    /// remain valid.
    pub stale_while_revalidate_seconds: u64,
}

impl Default for PersistenceConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            path: "amatl.sqlite3".into(),
            history_enabled: true,
            saved_document_max_bytes: 1_048_576,
            saved_documents_retention_days: 0,
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
    pub saved_documents_retention_days: Option<u32>,
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
        if let Some(value) = self.saved_documents_retention_days {
            config.saved_documents_retention_days = value;
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
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ProviderSearchCacheConfig {
    pub enabled: bool,
    pub ttl_seconds: u64,
    pub max_entries: u64,
    pub max_bytes: u64,
}

impl Config {
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
        if let Some(value) = patch.saved_documents_retention_days {
            table.insert(
                "saved_documents_retention_days",
                toml_edit::value(signed_integer(
                    value,
                    "persistence.saved_documents_retention_days",
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
        write_config_document(path, &document)?;
        Ok(())
    }
    /// Retention windows and backup limits for the SQLite persistence layer.
    pub(super) fn validate_persistence(&self) -> Result<(), ConfigError> {
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
        if self.persistence.saved_document_max_bytes == 0
            || self.persistence.saved_document_max_bytes > 16 * 1024 * 1024
        {
            return Err(ConfigError::Policy(
                "persistence.saved_document_max_bytes must be between 1 and 16777216".into(),
            ));
        }
        Ok(())
    }
    /// Provider-search and document cache limits.
    pub(super) fn validate_cache(&self) -> Result<(), ConfigError> {
        if self.cache.provider_search.ttl_seconds == 0
            || self.cache.provider_search.max_entries == 0
            || self.cache.provider_search.max_bytes == 0
            || self.cache.document.ttl_seconds == 0
            || self.cache.document.max_entries == 0
            || self.cache.document.max_bytes == 0
        {
            return Err(ConfigError::Policy("cache limits must be positive".into()));
        }
        Ok(())
    }
}
