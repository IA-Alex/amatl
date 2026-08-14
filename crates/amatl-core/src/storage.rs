use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::{Row, SqlitePool};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use thiserror::Error;

pub const MIGRATION_VERSION: i64 = 6;
const POOL_SIZE: u32 = 4;

#[derive(Clone)]
pub struct SqliteStorage {
    pool: SqlitePool,
    path: PathBuf,
    write_lock: Arc<tokio::sync::Mutex<()>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StorageHealth {
    pub path: PathBuf,
    pub journal_mode: String,
    pub synchronous: i64,
    pub busy_timeout_ms: i64,
    pub migration_version: i64,
    pub pool_size: u32,
}

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct CacheStats {
    pub entries: u64,
    pub size_bytes: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StoredTelemetryObservation {
    pub observed_at: i64,
    pub provider: String,
    pub category: String,
    pub outcome: String,
    pub latency_ms: u64,
    pub total_results: u64,
    pub unique_results: u64,
    pub duplicate_ratio: f64,
    pub top_k_contribution: f64,
    pub diversity: f64,
    pub cost_units: u64,
    pub request_id: Option<String>,
}

/// A recorded search in the user's history.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SearchHistoryEntry {
    pub id: i64,
    pub normalized_query: String,
    pub raw_query: String,
    pub category: Option<String>,
    pub provider_count: i64,
    pub total_results: i64,
    pub deep_fetches: i64,
    pub created_at: i64,
    pub surface: String,
}

/// A saved document persisted for cross-session reuse.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SavedDocument {
    pub id: i64,
    pub canonical_url: String,
    pub title: Option<String>,
    pub snippet: Option<String>,
    pub content_hash: String,
    pub extractor_version: String,
    pub payload: String,
    pub size_bytes: i64,
    pub saved_at: i64,
    pub source_query: Option<String>,
    pub tags: String,
}

/// Persisted circuit breaker row for one provider.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredCircuitRecord {
    pub provider: String,
    pub consecutive_failures: u32,
    pub opened_at: Option<i64>,
    pub open_until: Option<i64>,
    pub updated_at: i64,
}

/// Result of a conditional document cache lookup.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CachedDocument {
    pub payload: String,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    /// Whether the cached entry is still within its fresh TTL window.
    pub fresh: bool,
}

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("SQLite storage could not be opened")]
    Open,
    #[error("SQLite storage operation failed")]
    Operation,
    #[error("SQLite database was quarantined as corrupt")]
    Corrupt { quarantine_path: PathBuf },
    #[error("database version {db_version} is newer than code version {code_version}; downgrade is not supported")]
    IncompatibleVersion { db_version: i64, code_version: i64 },
}

impl SqliteStorage {
    pub async fn open(path: impl AsRef<Path>) -> Result<Self, StorageError> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent).map_err(|_| StorageError::Open)?;
        }
        quarantine_if_header_is_invalid(&path)?;

        let options = SqliteConnectOptions::new()
            .filename(&path)
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Normal)
            .busy_timeout(Duration::from_millis(5_000));
        let pool = match SqlitePoolOptions::new()
            .max_connections(POOL_SIZE)
            .acquire_timeout(Duration::from_millis(5_000))
            .connect_with(options)
            .await
        {
            Ok(pool) => pool,
            Err(error) if is_corruption(&error) => return quarantine(&path),
            Err(_) => return Err(StorageError::Open),
        };

        let integrity = sqlx::query_scalar::<_, String>("PRAGMA quick_check(1)")
            .fetch_one(&pool)
            .await;
        if !matches!(integrity.as_deref(), Ok("ok")) {
            pool.close().await;
            return quarantine(&path);
        }

        run_migrations(pool.clone(), path.clone()).await?;
        Ok(Self {
            pool,
            path,
            write_lock: Arc::new(tokio::sync::Mutex::new(())),
        })
    }

    pub async fn health(&self) -> Result<StorageHealth, StorageError> {
        Ok(StorageHealth {
            path: self.path.clone(),
            journal_mode: sqlx::query_scalar("PRAGMA journal_mode")
                .fetch_one(&self.pool)
                .await
                .map_err(|_| StorageError::Operation)?,
            synchronous: sqlx::query_scalar("PRAGMA synchronous")
                .fetch_one(&self.pool)
                .await
                .map_err(|_| StorageError::Operation)?,
            busy_timeout_ms: sqlx::query_scalar("PRAGMA busy_timeout")
                .fetch_one(&self.pool)
                .await
                .map_err(|_| StorageError::Operation)?,
            migration_version: sqlx::query_scalar("PRAGMA user_version")
                .fetch_one(&self.pool)
                .await
                .map_err(|_| StorageError::Operation)?,
            pool_size: POOL_SIZE,
        })
    }

    /// Run a downgrade migration to the specified target version.
    ///
    /// This is a destructive operation that should only be used when rolling back
    /// to an older version of the application. A backup is created automatically
    /// before the downgrade is applied.
    pub async fn downgrade_to(&self, target_version: i64) -> Result<(), StorageError> {
        let current: i64 = sqlx::query_scalar("PRAGMA user_version")
            .fetch_one(&self.pool)
            .await
            .map_err(operation)?;
        if target_version >= current {
            return Err(StorageError::Operation);
        }
        if !(0..MIGRATION_VERSION).contains(&target_version) {
            return Err(StorageError::Operation);
        }

        backup_database(&self.path)?;

        let _write_guard = self.write_lock.lock().await;
        for version in (target_version + 1..=current).rev() {
            let script = match version {
                6 => include_str!("../migrations/downgrade/0006_to_0005.sql"),
                5 => include_str!("../migrations/downgrade/0005_to_0004.sql"),
                4 => include_str!("../migrations/downgrade/0004_to_0003.sql"),
                3 => include_str!("../migrations/downgrade/0003_to_0002.sql"),
                2 => include_str!("../migrations/downgrade/0002_to_0001.sql"),
                _ => continue,
            };
            let mut transaction = self.pool.begin().await.map_err(operation)?;
            sqlx::raw_sql(script)
                .execute(transaction.as_mut())
                .await
                .map_err(operation)?;
            sqlx::query("DELETE FROM amatl_schema_migrations WHERE version = ?")
                .bind(version)
                .execute(transaction.as_mut())
                .await
                .map_err(operation)?;
            transaction.commit().await.map_err(operation)?;
        }
        Ok(())
    }

    /// Restore the database from a backup file.
    ///
    /// This closes the current pool, replaces the database file with the backup,
    /// and reopens the pool. The caller must obtain a new `SqliteStorage` handle
    /// after this operation.
    pub async fn restore_from_backup(
        path: impl AsRef<Path>,
        backup_path: impl AsRef<Path>,
    ) -> Result<Self, StorageError> {
        let path = path.as_ref();
        let backup_path = backup_path.as_ref();

        if !backup_path.exists() {
            return Err(StorageError::Open);
        }

        // Verify the backup is a valid SQLite database.
        quarantine_if_header_is_invalid(backup_path)?;

        // Create a safety backup of the current file if it exists.
        if path.exists() {
            let safety_path = path.with_extension(format!(
                "pre-restore-{}.sqlite3",
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map_or(0, |d| d.as_secs())
            ));
            std::fs::copy(path, &safety_path).map_err(|_| StorageError::Operation)?;
        }

        // Replace the database file with the backup.
        std::fs::copy(backup_path, path).map_err(|_| StorageError::Operation)?;

        // Open with the restored file.
        Self::open(path).await
    }

    /// List available backup files for a database path.
    pub fn list_backups(db_path: &Path) -> Result<Vec<PathBuf>, StorageError> {
        let dir = db_path.parent().unwrap_or(Path::new("."));
        let stem = db_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("amatl");
        let mut backups: Vec<PathBuf> = std::fs::read_dir(dir)
            .map_err(|_| StorageError::Operation)?
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .filter(|p| {
                p.is_file()
                    && p.file_name().and_then(|n| n.to_str()).is_some_and(|name| {
                        name.starts_with(stem)
                            && (name.contains("backup-") || name.contains("pre-restore-"))
                    })
            })
            .collect();
        backups.sort();
        Ok(backups)
    }

    pub(crate) async fn cache_get(
        &self,
        provider: &str,
        adapter_version: &str,
        normalized_query: &str,
        structured_filters: &str,
        now: i64,
        ttl_seconds: u64,
    ) -> Result<Option<String>, StorageError> {
        let _write_guard = self.write_lock.lock().await;
        let mut transaction = self.pool.begin().await.map_err(operation)?;
        let payload = sqlx::query_scalar::<_, String>(
            "SELECT payload FROM provider_search_cache
             WHERE provider = ? AND adapter_version = ? AND normalized_query = ?
               AND structured_filters = ? AND created_at >= ?",
        )
        .bind(provider)
        .bind(adapter_version)
        .bind(normalized_query)
        .bind(structured_filters)
        .bind(now.saturating_sub(ttl_seconds as i64))
        .fetch_optional(transaction.as_mut())
        .await
        .map_err(operation)?;
        if payload.is_some() {
            sqlx::query(
                "UPDATE provider_search_cache SET last_accessed = ?
                 WHERE provider = ? AND adapter_version = ? AND normalized_query = ?
                   AND structured_filters = ?",
            )
            .bind(now)
            .bind(provider)
            .bind(adapter_version)
            .bind(normalized_query)
            .bind(structured_filters)
            .execute(transaction.as_mut())
            .await
            .map_err(operation)?;
        }
        transaction.commit().await.map_err(operation)?;
        Ok(payload)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn cache_put(
        &self,
        provider: &str,
        adapter_version: &str,
        normalized_query: &str,
        structured_filters: &str,
        payload: &str,
        now: i64,
        ttl_seconds: u64,
        max_entries: u64,
        max_bytes: u64,
    ) -> Result<(), StorageError> {
        let _write_guard = self.write_lock.lock().await;
        let mut transaction = self.pool.begin().await.map_err(operation)?;
        sqlx::query(
            "INSERT INTO provider_search_cache
               (provider, adapter_version, normalized_query, structured_filters, payload,
                size_bytes, created_at, last_accessed)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(provider, adapter_version, normalized_query, structured_filters)
             DO UPDATE SET payload = excluded.payload, size_bytes = excluded.size_bytes,
               created_at = excluded.created_at, last_accessed = excluded.last_accessed",
        )
        .bind(provider)
        .bind(adapter_version)
        .bind(normalized_query)
        .bind(structured_filters)
        .bind(payload)
        .bind(payload.len() as i64)
        .bind(now)
        .bind(now)
        .execute(transaction.as_mut())
        .await
        .map_err(operation)?;
        sqlx::query("DELETE FROM provider_search_cache WHERE created_at < ?")
            .bind(now.saturating_sub(ttl_seconds as i64))
            .execute(transaction.as_mut())
            .await
            .map_err(operation)?;

        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM provider_search_cache")
            .fetch_one(transaction.as_mut())
            .await
            .map_err(operation)?;
        if count > max_entries as i64 {
            sqlx::query(
                "DELETE FROM provider_search_cache WHERE rowid IN
                 (SELECT rowid FROM provider_search_cache
                  ORDER BY last_accessed ASC, rowid ASC LIMIT ?)",
            )
            .bind(count - max_entries as i64)
            .execute(transaction.as_mut())
            .await
            .map_err(operation)?;
        }
        loop {
            let size: i64 = sqlx::query_scalar(
                "SELECT COALESCE(SUM(size_bytes), 0) FROM provider_search_cache",
            )
            .fetch_one(transaction.as_mut())
            .await
            .map_err(operation)?;
            if size <= max_bytes as i64 {
                break;
            }
            let deleted = sqlx::query(
                "DELETE FROM provider_search_cache WHERE rowid =
                 (SELECT rowid FROM provider_search_cache
                  ORDER BY last_accessed ASC, rowid ASC LIMIT 1)",
            )
            .execute(transaction.as_mut())
            .await
            .map_err(operation)?
            .rows_affected();
            if deleted == 0 {
                break;
            }
        }
        transaction.commit().await.map_err(operation)?;
        Ok(())
    }

    pub(crate) async fn cache_stats(&self) -> Result<CacheStats, StorageError> {
        let row = sqlx::query(
            "SELECT COUNT(*) AS entries, COALESCE(SUM(size_bytes), 0) AS size_bytes
             FROM provider_search_cache",
        )
        .fetch_one(&self.pool)
        .await
        .map_err(operation)?;
        Ok(CacheStats {
            entries: row.get::<i64, _>("entries").max(0) as u64,
            size_bytes: row.get::<i64, _>("size_bytes").max(0) as u64,
        })
    }

    pub(crate) async fn cache_purge(&self) -> Result<u64, StorageError> {
        let _write_guard = self.write_lock.lock().await;
        Ok(sqlx::query("DELETE FROM provider_search_cache")
            .execute(&self.pool)
            .await
            .map_err(operation)?
            .rows_affected())
    }

    pub(crate) async fn document_cache_get(
        &self,
        canonical_url: &str,
        content_hash: &str,
        extractor_version: &str,
        now: i64,
        ttl_seconds: u64,
    ) -> Result<Option<String>, StorageError> {
        let _write_guard = self.write_lock.lock().await;
        let mut transaction = self.pool.begin().await.map_err(operation)?;
        let payload = sqlx::query_scalar::<_, String>(
            "SELECT payload FROM document_cache
             WHERE canonical_url = ? AND content_hash = ? AND extractor_version = ?
               AND created_at >= ?",
        )
        .bind(canonical_url)
        .bind(content_hash)
        .bind(extractor_version)
        .bind(now.saturating_sub(ttl_seconds as i64))
        .fetch_optional(transaction.as_mut())
        .await
        .map_err(operation)?;
        if payload.is_some() {
            sqlx::query(
                "UPDATE document_cache SET last_accessed = ?
                 WHERE canonical_url = ? AND content_hash = ? AND extractor_version = ?",
            )
            .bind(now)
            .bind(canonical_url)
            .bind(content_hash)
            .bind(extractor_version)
            .execute(transaction.as_mut())
            .await
            .map_err(operation)?;
        }
        transaction.commit().await.map_err(operation)?;
        Ok(payload)
    }

    pub(crate) async fn document_cache_get_latest(
        &self,
        canonical_url: &str,
        extractor_version: &str,
        now: i64,
        ttl_seconds: u64,
    ) -> Result<Option<String>, StorageError> {
        let _write_guard = self.write_lock.lock().await;
        let payload = sqlx::query_scalar::<_, String>(
            "SELECT payload FROM document_cache
             WHERE canonical_url = ? AND extractor_version = ? AND created_at >= ?
             ORDER BY created_at DESC, rowid DESC LIMIT 1",
        )
        .bind(canonical_url)
        .bind(extractor_version)
        .bind(now.saturating_sub(ttl_seconds as i64))
        .fetch_optional(&self.pool)
        .await
        .map_err(operation)?;
        if payload.is_some() {
            sqlx::query(
                "UPDATE document_cache SET last_accessed = ?
                 WHERE rowid = (SELECT rowid FROM document_cache
                   WHERE canonical_url = ? AND extractor_version = ? AND created_at >= ?
                   ORDER BY created_at DESC, rowid DESC LIMIT 1)",
            )
            .bind(now)
            .bind(canonical_url)
            .bind(extractor_version)
            .bind(now.saturating_sub(ttl_seconds as i64))
            .execute(&self.pool)
            .await
            .map_err(operation)?;
        }
        Ok(payload)
    }

    /// Get a cached document with its revalidation headers.
    ///
    /// Returns the payload along with any stored ETag and Last-Modified values
    /// so the caller can perform conditional revalidation against the origin.
    pub(crate) async fn document_cache_get_with_revalidation(
        &self,
        canonical_url: &str,
        content_hash: &str,
        extractor_version: &str,
        now: i64,
        ttl_seconds: u64,
        stale_while_revalidate_seconds: u64,
    ) -> Result<Option<CachedDocument>, StorageError> {
        let _write_guard = self.write_lock.lock().await;
        let mut transaction = self.pool.begin().await.map_err(operation)?;
        let fresh_cutoff = now.saturating_sub(ttl_seconds as i64);
        let stale_cutoff =
            now.saturating_sub((ttl_seconds + stale_while_revalidate_seconds) as i64);

        let row = sqlx::query(
            "SELECT payload, etag, last_modified, created_at FROM document_cache
             WHERE canonical_url = ? AND content_hash = ? AND extractor_version = ?
               AND created_at >= ?
             ORDER BY created_at DESC LIMIT 1",
        )
        .bind(canonical_url)
        .bind(content_hash)
        .bind(extractor_version)
        .bind(stale_cutoff)
        .fetch_optional(transaction.as_mut())
        .await
        .map_err(operation)?;

        let Some(row) = row else {
            return Ok(None);
        };

        let payload: String = row.get("payload");
        let etag: Option<String> = row.get("etag");
        let last_modified: Option<String> = row.get("last_modified");
        let created_at: i64 = row.get("created_at");

        // Update last_accessed for LRU tracking.
        sqlx::query(
            "UPDATE document_cache SET last_accessed = ?
             WHERE canonical_url = ? AND content_hash = ? AND extractor_version = ?",
        )
        .bind(now)
        .bind(canonical_url)
        .bind(content_hash)
        .bind(extractor_version)
        .execute(transaction.as_mut())
        .await
        .map_err(operation)?;

        transaction.commit().await.map_err(operation)?;

        Ok(Some(CachedDocument {
            payload,
            etag,
            last_modified,
            fresh: created_at >= fresh_cutoff,
        }))
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn document_cache_put(
        &self,
        canonical_url: &str,
        content_hash: &str,
        extractor_version: &str,
        payload: &str,
        now: i64,
        ttl_seconds: u64,
        max_entries: u64,
        max_bytes: u64,
        etag: Option<&str>,
        last_modified: Option<&str>,
    ) -> Result<(), StorageError> {
        let _write_guard = self.write_lock.lock().await;
        let mut transaction = self.pool.begin().await.map_err(operation)?;
        sqlx::query(
            "INSERT INTO document_cache
             (canonical_url, content_hash, extractor_version, payload, size_bytes,
              created_at, last_accessed, etag, last_modified)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(canonical_url, content_hash, extractor_version) DO UPDATE SET
               payload = excluded.payload, size_bytes = excluded.size_bytes,
               created_at = excluded.created_at, last_accessed = excluded.last_accessed,
               etag = excluded.etag, last_modified = excluded.last_modified",
        )
        .bind(canonical_url)
        .bind(content_hash)
        .bind(extractor_version)
        .bind(payload)
        .bind(payload.len() as i64)
        .bind(now)
        .bind(now)
        .bind(etag)
        .bind(last_modified)
        .execute(transaction.as_mut())
        .await
        .map_err(operation)?;
        sqlx::query("DELETE FROM document_cache WHERE created_at < ?")
            .bind(now.saturating_sub(ttl_seconds as i64))
            .execute(transaction.as_mut())
            .await
            .map_err(operation)?;
        sqlx::query(
            "DELETE FROM document_cache WHERE rowid IN
             (SELECT rowid FROM document_cache ORDER BY last_accessed ASC, rowid ASC
              LIMIT MAX(0, (SELECT COUNT(*) FROM document_cache) - ?))",
        )
        .bind(max_entries as i64)
        .execute(transaction.as_mut())
        .await
        .map_err(operation)?;
        loop {
            let size: i64 =
                sqlx::query_scalar("SELECT COALESCE(SUM(size_bytes), 0) FROM document_cache")
                    .fetch_one(transaction.as_mut())
                    .await
                    .map_err(operation)?;
            if size <= max_bytes as i64 {
                break;
            }
            let deleted = sqlx::query(
                "DELETE FROM document_cache WHERE rowid =
                 (SELECT rowid FROM document_cache ORDER BY last_accessed ASC, rowid ASC LIMIT 1)",
            )
            .execute(transaction.as_mut())
            .await
            .map_err(operation)?
            .rows_affected();
            if deleted == 0 {
                break;
            }
        }
        transaction.commit().await.map_err(operation)?;
        Ok(())
    }

    pub(crate) async fn document_cache_stats(&self) -> Result<CacheStats, StorageError> {
        let row = sqlx::query(
            "SELECT COUNT(*) AS entries, COALESCE(SUM(size_bytes), 0) AS size_bytes FROM document_cache",
        ).fetch_one(&self.pool).await.map_err(operation)?;
        Ok(CacheStats {
            entries: row.get::<i64, _>("entries").max(0) as u64,
            size_bytes: row.get::<i64, _>("size_bytes").max(0) as u64,
        })
    }

    pub(crate) async fn document_cache_purge(&self) -> Result<u64, StorageError> {
        let _write_guard = self.write_lock.lock().await;
        Ok(sqlx::query("DELETE FROM document_cache")
            .execute(&self.pool)
            .await
            .map_err(operation)?
            .rows_affected())
    }

    pub(crate) async fn telemetry_insert(
        &self,
        observation: &StoredTelemetryObservation,
    ) -> Result<(), StorageError> {
        let _write_guard = self.write_lock.lock().await;
        sqlx::query(
            "INSERT INTO telemetry_observations
             (observed_at, provider, category, outcome, latency_ms, total_results,
              unique_results, duplicate_ratio, top_k_contribution, diversity, cost_units,
              request_id)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(observation.observed_at)
        .bind(&observation.provider)
        .bind(&observation.category)
        .bind(&observation.outcome)
        .bind(observation.latency_ms as i64)
        .bind(observation.total_results as i64)
        .bind(observation.unique_results as i64)
        .bind(observation.duplicate_ratio)
        .bind(observation.top_k_contribution)
        .bind(observation.diversity)
        .bind(observation.cost_units as i64)
        .bind(&observation.request_id)
        .execute(&self.pool)
        .await
        .map_err(operation)?;
        Ok(())
    }

    pub(crate) async fn telemetry_load(
        &self,
        since: i64,
    ) -> Result<Vec<StoredTelemetryObservation>, StorageError> {
        let rows = sqlx::query(
            "SELECT observed_at, provider, category, outcome, latency_ms, total_results,
                    unique_results, duplicate_ratio, top_k_contribution, diversity, cost_units,
                    request_id
             FROM telemetry_observations WHERE observed_at >= ?
             ORDER BY observed_at ASC, id ASC",
        )
        .bind(since)
        .fetch_all(&self.pool)
        .await
        .map_err(operation)?;
        Ok(rows
            .into_iter()
            .map(|row| StoredTelemetryObservation {
                observed_at: row.get("observed_at"),
                provider: row.get("provider"),
                category: row.get("category"),
                outcome: row.get("outcome"),
                latency_ms: row.get::<i64, _>("latency_ms").max(0) as u64,
                total_results: row.get::<i64, _>("total_results").max(0) as u64,
                unique_results: row.get::<i64, _>("unique_results").max(0) as u64,
                duplicate_ratio: row.get("duplicate_ratio"),
                top_k_contribution: row.get("top_k_contribution"),
                diversity: row.get("diversity"),
                cost_units: row.get::<i64, _>("cost_units").max(0) as u64,
                request_id: row.get("request_id"),
            })
            .collect())
    }

    pub(crate) async fn telemetry_prune(&self, before: i64) -> Result<u64, StorageError> {
        let _write_guard = self.write_lock.lock().await;
        Ok(
            sqlx::query("DELETE FROM telemetry_observations WHERE observed_at < ?")
                .bind(before)
                .execute(&self.pool)
                .await
                .map_err(operation)?
                .rows_affected(),
        )
    }

    // ── Domain persistence: search history ──────────────────────────────

    /// Record a search in the history.
    #[allow(clippy::too_many_arguments)]
    pub async fn search_history_insert(
        &self,
        normalized_query: &str,
        raw_query: &str,
        category: Option<&str>,
        provider_count: i64,
        total_results: i64,
        deep_fetches: i64,
        surface: &str,
    ) -> Result<i64, StorageError> {
        let _write_guard = self.write_lock.lock().await;
        let now = now_unix();
        let id = sqlx::query(
            "INSERT INTO search_history
               (normalized_query, raw_query, category, provider_count, total_results,
                deep_fetches, created_at, surface)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(normalized_query)
        .bind(raw_query)
        .bind(category)
        .bind(provider_count)
        .bind(total_results)
        .bind(deep_fetches)
        .bind(now)
        .bind(surface)
        .execute(&self.pool)
        .await
        .map_err(operation)?
        .last_insert_rowid();
        Ok(id)
    }

    /// List recent search history entries, newest first.
    pub async fn search_history_list(
        &self,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<SearchHistoryEntry>, StorageError> {
        let rows = sqlx::query(
            "SELECT id, normalized_query, raw_query, category, provider_count,
                    total_results, deep_fetches, created_at, surface
             FROM search_history
             ORDER BY created_at DESC, id DESC
             LIMIT ? OFFSET ?",
        )
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await
        .map_err(operation)?;
        Ok(rows
            .into_iter()
            .map(|row| SearchHistoryEntry {
                id: row.get("id"),
                normalized_query: row.get("normalized_query"),
                raw_query: row.get("raw_query"),
                category: row.get("category"),
                provider_count: row.get("provider_count"),
                total_results: row.get("total_results"),
                deep_fetches: row.get("deep_fetches"),
                created_at: row.get("created_at"),
                surface: row.get("surface"),
            })
            .collect())
    }

    /// Search history entries matching a normalized query prefix.
    pub async fn search_history_find(
        &self,
        normalized_query_prefix: &str,
        limit: i64,
    ) -> Result<Vec<SearchHistoryEntry>, StorageError> {
        let rows = sqlx::query(
            "SELECT id, normalized_query, raw_query, category, provider_count,
                    total_results, deep_fetches, created_at, surface
             FROM search_history
             WHERE normalized_query LIKE ? || '%'
             ORDER BY created_at DESC, id DESC
             LIMIT ?",
        )
        .bind(normalized_query_prefix)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(operation)?;
        Ok(rows
            .into_iter()
            .map(|row| SearchHistoryEntry {
                id: row.get("id"),
                normalized_query: row.get("normalized_query"),
                raw_query: row.get("raw_query"),
                category: row.get("category"),
                provider_count: row.get("provider_count"),
                total_results: row.get("total_results"),
                deep_fetches: row.get("deep_fetches"),
                created_at: row.get("created_at"),
                surface: row.get("surface"),
            })
            .collect())
    }

    /// Delete a search history entry by id.
    pub async fn search_history_delete(&self, id: i64) -> Result<bool, StorageError> {
        let _write_guard = self.write_lock.lock().await;
        let rows = sqlx::query("DELETE FROM search_history WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(operation)?
            .rows_affected();
        Ok(rows > 0)
    }

    /// Count recorded search history entries.
    pub async fn search_history_count(&self) -> Result<i64, StorageError> {
        sqlx::query_scalar("SELECT COUNT(*) FROM search_history")
            .fetch_one(&self.pool)
            .await
            .map_err(operation)
    }

    /// Purge all search history entries.
    pub async fn search_history_purge(&self) -> Result<u64, StorageError> {
        let _write_guard = self.write_lock.lock().await;
        Ok(sqlx::query("DELETE FROM search_history")
            .execute(&self.pool)
            .await
            .map_err(operation)?
            .rows_affected())
    }

    // ── Domain persistence: saved documents ─────────────────────────────

    /// Save a document for cross-session reuse.
    #[allow(clippy::too_many_arguments)]
    pub async fn saved_document_put(
        &self,
        canonical_url: &str,
        title: Option<&str>,
        snippet: Option<&str>,
        content_hash: &str,
        extractor_version: &str,
        payload: &str,
        source_query: Option<&str>,
        tags: &str,
    ) -> Result<i64, StorageError> {
        let _write_guard = self.write_lock.lock().await;
        let now = now_unix();
        let size_bytes = payload.len() as i64;
        let id = sqlx::query(
            "INSERT INTO saved_documents
               (canonical_url, title, snippet, content_hash, extractor_version,
                payload, size_bytes, saved_at, source_query, tags)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(canonical_url, content_hash, extractor_version)
             DO UPDATE SET title = excluded.title, snippet = excluded.snippet,
               payload = excluded.payload, size_bytes = excluded.size_bytes,
               saved_at = excluded.saved_at, source_query = excluded.source_query,
               tags = excluded.tags",
        )
        .bind(canonical_url)
        .bind(title)
        .bind(snippet)
        .bind(content_hash)
        .bind(extractor_version)
        .bind(payload)
        .bind(size_bytes)
        .bind(now)
        .bind(source_query)
        .bind(tags)
        .execute(&self.pool)
        .await
        .map_err(operation)?
        .last_insert_rowid();
        Ok(id)
    }

    /// Retrieve a saved document by URL, hash and extractor version.
    pub async fn saved_document_get(
        &self,
        canonical_url: &str,
        content_hash: &str,
        extractor_version: &str,
    ) -> Result<Option<SavedDocument>, StorageError> {
        let row = sqlx::query(
            "SELECT id, canonical_url, title, snippet, content_hash, extractor_version,
                    payload, size_bytes, saved_at, source_query, tags
             FROM saved_documents
             WHERE canonical_url = ? AND content_hash = ? AND extractor_version = ?",
        )
        .bind(canonical_url)
        .bind(content_hash)
        .bind(extractor_version)
        .fetch_optional(&self.pool)
        .await
        .map_err(operation)?;
        Ok(row.map(|row| SavedDocument {
            id: row.get("id"),
            canonical_url: row.get("canonical_url"),
            title: row.get("title"),
            snippet: row.get("snippet"),
            content_hash: row.get("content_hash"),
            extractor_version: row.get("extractor_version"),
            payload: row.get("payload"),
            size_bytes: row.get("size_bytes"),
            saved_at: row.get("saved_at"),
            source_query: row.get("source_query"),
            tags: row.get("tags"),
        }))
    }

    /// List saved documents, newest first.
    pub async fn saved_document_list(
        &self,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<SavedDocument>, StorageError> {
        let rows = sqlx::query(
            "SELECT id, canonical_url, title, snippet, content_hash, extractor_version,
                    payload, size_bytes, saved_at, source_query, tags
             FROM saved_documents
             ORDER BY saved_at DESC, id DESC
             LIMIT ? OFFSET ?",
        )
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await
        .map_err(operation)?;
        Ok(rows
            .into_iter()
            .map(|row| SavedDocument {
                id: row.get("id"),
                canonical_url: row.get("canonical_url"),
                title: row.get("title"),
                snippet: row.get("snippet"),
                content_hash: row.get("content_hash"),
                extractor_version: row.get("extractor_version"),
                payload: row.get("payload"),
                size_bytes: row.get("size_bytes"),
                saved_at: row.get("saved_at"),
                source_query: row.get("source_query"),
                tags: row.get("tags"),
            })
            .collect())
    }

    /// Delete a saved document by id.
    pub async fn saved_document_delete(&self, id: i64) -> Result<bool, StorageError> {
        let _write_guard = self.write_lock.lock().await;
        let rows = sqlx::query("DELETE FROM saved_documents WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(operation)?
            .rows_affected();
        Ok(rows > 0)
    }

    // ── Provider circuit breaker ────────────────────────────────────────

    /// Persist the breaker state of one provider.
    pub async fn circuit_put(&self, record: &StoredCircuitRecord) -> Result<(), StorageError> {
        let _write_guard = self.write_lock.lock().await;
        sqlx::query(
            "INSERT INTO provider_circuit
               (provider, consecutive_failures, opened_at, open_until, updated_at)
             VALUES (?, ?, ?, ?, ?)
             ON CONFLICT(provider) DO UPDATE SET
               consecutive_failures = excluded.consecutive_failures,
               opened_at = excluded.opened_at,
               open_until = excluded.open_until,
               updated_at = excluded.updated_at",
        )
        .bind(&record.provider)
        .bind(record.consecutive_failures as i64)
        .bind(record.opened_at)
        .bind(record.open_until)
        .bind(record.updated_at)
        .execute(&self.pool)
        .await
        .map_err(operation)?;
        Ok(())
    }

    /// Load every persisted breaker row.
    pub async fn circuit_load(&self) -> Result<Vec<StoredCircuitRecord>, StorageError> {
        let rows = sqlx::query(
            "SELECT provider, consecutive_failures, opened_at, open_until, updated_at
             FROM provider_circuit
             ORDER BY provider ASC",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(operation)?;
        Ok(rows
            .into_iter()
            .map(|row| StoredCircuitRecord {
                provider: row.get("provider"),
                consecutive_failures: row.get::<i64, _>("consecutive_failures").max(0) as u32,
                opened_at: row.get("opened_at"),
                open_until: row.get("open_until"),
                updated_at: row.get("updated_at"),
            })
            .collect())
    }

    /// Forget every persisted breaker row.
    pub async fn circuit_clear(&self) -> Result<u64, StorageError> {
        let _write_guard = self.write_lock.lock().await;
        Ok(sqlx::query("DELETE FROM provider_circuit")
            .execute(&self.pool)
            .await
            .map_err(operation)?
            .rows_affected())
    }

    /// Count saved documents.
    pub async fn saved_document_count(&self) -> Result<i64, StorageError> {
        sqlx::query_scalar("SELECT COUNT(*) FROM saved_documents")
            .fetch_one(&self.pool)
            .await
            .map_err(operation)
    }
}

impl StorageError {
    pub fn quarantine_path(&self) -> Option<&Path> {
        match self {
            Self::Corrupt { quarantine_path } => Some(quarantine_path),
            Self::Open | Self::Operation | Self::IncompatibleVersion { .. } => None,
        }
    }
}

fn operation(_: sqlx::Error) -> StorageError {
    StorageError::Operation
}

/// Apply pending migrations.
///
/// `db_path` is owned rather than borrowed on purpose: a `&Path` held across
/// the migration awaits makes the resulting future's `Send` bound unprovable
/// for callers in a generic position, such as the HTTP reload handler.
async fn run_migrations(pool: SqlitePool, db_path: PathBuf) -> Result<(), StorageError> {
    let mut transaction = pool.begin().await.map_err(operation)?;

    // Check database version compatibility before any migration.
    let db_version: i64 = sqlx::query_scalar("PRAGMA user_version")
        .fetch_one(transaction.as_mut())
        .await
        .map_err(operation)?;
    if db_version > MIGRATION_VERSION {
        return Err(StorageError::IncompatibleVersion {
            db_version,
            code_version: MIGRATION_VERSION,
        });
    }

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS amatl_schema_migrations (
           version INTEGER PRIMARY KEY,
           name TEXT NOT NULL,
           applied_at INTEGER NOT NULL
         )",
    )
    .execute(transaction.as_mut())
    .await
    .map_err(operation)?;

    // Determine which migrations need to be applied.
    let pending: Vec<_> = [
        (
            1_i64,
            "phase3_persistence",
            include_str!("../migrations/0001_phase3_persistence.sql"),
        ),
        (
            2_i64,
            "phase5_document_cache",
            include_str!("../migrations/0002_phase5_document_cache.sql"),
        ),
        (
            3_i64,
            "domain_persistence",
            include_str!("../migrations/0003_domain_persistence.sql"),
        ),
        (
            4_i64,
            "document_cache_revalidation",
            include_str!("../migrations/0004_document_cache_revalidation.sql"),
        ),
        (
            5_i64,
            "request_id_telemetry",
            include_str!("../migrations/0005_request_id.sql"),
        ),
        (
            6_i64,
            "provider_circuit",
            include_str!("../migrations/0006_provider_circuit.sql"),
        ),
    ]
    .into_iter()
    .filter(|(version, _, _)| *version > db_version)
    .collect();

    // Create a backup before applying any pending migrations.
    if !pending.is_empty() {
        backup_database(&db_path)?;
    }

    for (version, name, migration) in pending {
        let applied = sqlx::query_scalar::<_, i64>(
            "SELECT version FROM amatl_schema_migrations WHERE version = ?",
        )
        .bind(version)
        .fetch_optional(transaction.as_mut())
        .await
        .map_err(operation)?;
        if applied.is_none() {
            sqlx::raw_sql(migration)
                .execute(transaction.as_mut())
                .await
                .map_err(operation)?;
            sqlx::query(
                "INSERT INTO amatl_schema_migrations(version, name, applied_at) VALUES (?, ?, ?)",
            )
            .bind(version)
            .bind(name)
            .bind(now_unix())
            .execute(transaction.as_mut())
            .await
            .map_err(operation)?;
        }
    }
    transaction.commit().await.map_err(operation)?;
    Ok(())
}

/// Create a timestamped backup of the database file before migration.
fn backup_database(path: &Path) -> Result<(), StorageError> {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let backup_path = path.with_extension(format!("backup-{}.sqlite3", timestamp));
    std::fs::copy(path, &backup_path).map_err(|_| StorageError::Operation)?;
    tracing::info!(
        target: "amatl::storage",
        path = %path.display(),
        backup = %backup_path.display(),
        "database backup created before migration"
    );
    Ok(())
}

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs() as i64)
}

fn quarantine_if_header_is_invalid(path: &Path) -> Result<(), StorageError> {
    if !path.exists() {
        return Ok(());
    }
    let mut file = std::fs::File::open(path).map_err(|_| StorageError::Open)?;
    let metadata = file.metadata().map_err(|_| StorageError::Open)?;
    if !metadata.is_file() {
        return Err(StorageError::Open);
    }
    let size = metadata.len();
    if size == 0 {
        return Ok(());
    }
    let mut header = [0_u8; 16];
    if file.read_exact(&mut header).is_err() || &header != b"SQLite format 3\0" {
        return quarantine(path);
    }
    Ok(())
}

fn is_corruption(error: &sqlx::Error) -> bool {
    matches!(
        error,
        sqlx::Error::Database(database)
            if matches!(database.code().as_deref(), Some("11" | "26"))
    )
}

fn quarantine<T>(path: &Path) -> Result<T, StorageError> {
    let quarantine_path = quarantine_path(path);
    std::fs::rename(path, &quarantine_path).map_err(|_| StorageError::Operation)?;
    Err(StorageError::Corrupt { quarantine_path })
}

fn quarantine_path(path: &Path) -> PathBuf {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs());
    let mut value = path.as_os_str().to_os_string();
    value.push(format!(".corrupt-{timestamp}"));
    PathBuf::from(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "amatl-{name}-{}-{}.sqlite3",
            std::process::id(),
            now()
        ))
    }

    fn now() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos() as u64
    }

    #[tokio::test]
    async fn configures_wal_busy_timeout_and_versioned_migration() {
        let path = path("health");
        let storage = SqliteStorage::open(&path).await.unwrap();
        let health = storage.health().await.unwrap();
        assert_eq!(health.journal_mode.to_ascii_lowercase(), "wal");
        assert_eq!(health.synchronous, 1);
        assert_eq!(health.busy_timeout_ms, 5_000);
        assert_eq!(health.migration_version, MIGRATION_VERSION);
        assert_eq!(health.pool_size, POOL_SIZE);
        storage.pool.close().await;

        let reopened = SqliteStorage::open(&path).await.unwrap();
        assert_eq!(
            reopened.health().await.unwrap().migration_version,
            MIGRATION_VERSION
        );
        reopened.pool.close().await;
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn cache_enforces_ttl_and_lru_entry_limit() {
        let storage = SqliteStorage::open(path("cache-policy")).await.unwrap();
        storage
            .cache_put("p", "v1", "q1", "{}", "one", 100, 10, 1, 1_000)
            .await
            .unwrap();
        assert!(storage
            .cache_get("p", "v1", "q1", "{}", 111, 10)
            .await
            .unwrap()
            .is_none());

        storage
            .cache_put("p", "v1", "q1", "{}", "one", 200, 100, 1, 1_000)
            .await
            .unwrap();
        storage
            .cache_put("p", "v1", "q2", "{}", "two", 201, 100, 1, 1_000)
            .await
            .unwrap();
        assert_eq!(storage.cache_stats().await.unwrap().entries, 1);
        assert!(storage
            .cache_get("p", "v1", "q1", "{}", 202, 100)
            .await
            .unwrap()
            .is_none());
        assert!(storage
            .cache_get("p", "v1", "q2", "{}", 202, 100)
            .await
            .unwrap()
            .is_some());
    }

    #[tokio::test]
    async fn cache_enforces_byte_quota() {
        let storage = SqliteStorage::open(path("cache-quota")).await.unwrap();
        storage
            .cache_put("p", "v1", "q", "{}", "four", 100, 100, 10, 3)
            .await
            .unwrap();
        assert_eq!(storage.cache_stats().await.unwrap(), CacheStats::default());
    }

    #[tokio::test]
    async fn document_cache_is_versioned_and_enforces_lru_quota() {
        let storage = SqliteStorage::open(path("document-cache-policy"))
            .await
            .unwrap();
        storage
            .document_cache_put(
                "https://a.test",
                "h1",
                "e1",
                "one",
                100,
                100,
                1,
                1_000,
                None,
                None,
            )
            .await
            .unwrap();
        storage
            .document_cache_put(
                "https://b.test",
                "h2",
                "e1",
                "two",
                101,
                100,
                1,
                1_000,
                None,
                None,
            )
            .await
            .unwrap();
        assert_eq!(storage.document_cache_stats().await.unwrap().entries, 1);
        assert!(storage
            .document_cache_get("https://a.test", "h1", "e1", 102, 100)
            .await
            .unwrap()
            .is_none());
        assert!(storage
            .document_cache_get("https://b.test", "h2", "e2", 102, 100)
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn document_cache_put_purges_expired_entries() {
        let storage = SqliteStorage::open(path("document-cache-ttl"))
            .await
            .unwrap();
        storage
            .document_cache_put(
                "https://old.test",
                "h1",
                "e1",
                "old",
                100,
                10,
                10,
                1_000,
                None,
                None,
            )
            .await
            .unwrap();
        storage
            .document_cache_put(
                "https://new.test",
                "h2",
                "e1",
                "new",
                200,
                10,
                10,
                1_000,
                None,
                None,
            )
            .await
            .unwrap();
        assert_eq!(storage.document_cache_stats().await.unwrap().entries, 1);
        assert!(storage
            .document_cache_get_latest("https://old.test", "e1", 200, 10)
            .await
            .unwrap()
            .is_none());
    }
}
