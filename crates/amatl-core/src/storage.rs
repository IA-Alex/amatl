use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::{Row, SqlitePool};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use thiserror::Error;

pub const MIGRATION_VERSION: i64 = 2;
const POOL_SIZE: u32 = 4;

#[derive(Clone)]
pub struct SqliteStorage {
    pool: SqlitePool,
    path: PathBuf,
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

#[derive(Clone, Debug, Default, PartialEq, Eq)]
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
}

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("SQLite storage could not be opened")]
    Open,
    #[error("SQLite storage operation failed")]
    Operation,
    #[error("SQLite database was quarantined as corrupt")]
    Corrupt { quarantine_path: PathBuf },
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

        run_migrations(&pool).await?;
        Ok(Self { pool, path })
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

    pub(crate) async fn cache_get(
        &self,
        provider: &str,
        adapter_version: &str,
        normalized_query: &str,
        structured_filters: &str,
        now: i64,
        ttl_seconds: u64,
    ) -> Result<Option<String>, StorageError> {
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
        .fetch_optional(&mut *transaction)
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
            .execute(&mut *transaction)
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
        .execute(&mut *transaction)
        .await
        .map_err(operation)?;
        sqlx::query("DELETE FROM provider_search_cache WHERE created_at < ?")
            .bind(now.saturating_sub(ttl_seconds as i64))
            .execute(&mut *transaction)
            .await
            .map_err(operation)?;

        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM provider_search_cache")
            .fetch_one(&mut *transaction)
            .await
            .map_err(operation)?;
        if count > max_entries as i64 {
            sqlx::query(
                "DELETE FROM provider_search_cache WHERE rowid IN
                 (SELECT rowid FROM provider_search_cache
                  ORDER BY last_accessed ASC, rowid ASC LIMIT ?)",
            )
            .bind(count - max_entries as i64)
            .execute(&mut *transaction)
            .await
            .map_err(operation)?;
        }
        loop {
            let size: i64 = sqlx::query_scalar(
                "SELECT COALESCE(SUM(size_bytes), 0) FROM provider_search_cache",
            )
            .fetch_one(&mut *transaction)
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
            .execute(&mut *transaction)
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
        .fetch_optional(&mut *transaction)
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
            .execute(&mut *transaction)
            .await
            .map_err(operation)?;
        }
        transaction.commit().await.map_err(operation)?;
        Ok(payload)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn document_cache_put(
        &self,
        canonical_url: &str,
        content_hash: &str,
        extractor_version: &str,
        payload: &str,
        now: i64,
        max_entries: u64,
        max_bytes: u64,
    ) -> Result<(), StorageError> {
        let mut transaction = self.pool.begin().await.map_err(operation)?;
        sqlx::query(
            "INSERT INTO document_cache
             (canonical_url, content_hash, extractor_version, payload, size_bytes, created_at, last_accessed)
             VALUES (?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(canonical_url, content_hash, extractor_version) DO UPDATE SET
               payload = excluded.payload, size_bytes = excluded.size_bytes,
               created_at = excluded.created_at, last_accessed = excluded.last_accessed",
        )
        .bind(canonical_url)
        .bind(content_hash)
        .bind(extractor_version)
        .bind(payload)
        .bind(payload.len() as i64)
        .bind(now)
        .bind(now)
        .execute(&mut *transaction)
        .await
        .map_err(operation)?;
        sqlx::query(
            "DELETE FROM document_cache WHERE rowid IN
             (SELECT rowid FROM document_cache ORDER BY last_accessed ASC, rowid ASC
              LIMIT MAX(0, (SELECT COUNT(*) FROM document_cache) - ?))",
        )
        .bind(max_entries as i64)
        .execute(&mut *transaction)
        .await
        .map_err(operation)?;
        loop {
            let size: i64 =
                sqlx::query_scalar("SELECT COALESCE(SUM(size_bytes), 0) FROM document_cache")
                    .fetch_one(&mut *transaction)
                    .await
                    .map_err(operation)?;
            if size <= max_bytes as i64 {
                break;
            }
            let deleted = sqlx::query(
                "DELETE FROM document_cache WHERE rowid =
                 (SELECT rowid FROM document_cache ORDER BY last_accessed ASC, rowid ASC LIMIT 1)",
            )
            .execute(&mut *transaction)
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
        sqlx::query(
            "INSERT INTO telemetry_observations
             (observed_at, provider, category, outcome, latency_ms, total_results,
              unique_results, duplicate_ratio, top_k_contribution, diversity, cost_units)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
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
                    unique_results, duplicate_ratio, top_k_contribution, diversity, cost_units
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
            })
            .collect())
    }

    pub(crate) async fn telemetry_prune(&self, before: i64) -> Result<u64, StorageError> {
        Ok(
            sqlx::query("DELETE FROM telemetry_observations WHERE observed_at < ?")
                .bind(before)
                .execute(&self.pool)
                .await
                .map_err(operation)?
                .rows_affected(),
        )
    }
}

fn operation(_: sqlx::Error) -> StorageError {
    StorageError::Operation
}

async fn run_migrations(pool: &SqlitePool) -> Result<(), StorageError> {
    let mut transaction = pool.begin().await.map_err(operation)?;
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS amatl_schema_migrations (
           version INTEGER PRIMARY KEY,
           name TEXT NOT NULL,
           applied_at INTEGER NOT NULL
         )",
    )
    .execute(&mut *transaction)
    .await
    .map_err(operation)?;
    for (version, name, migration) in [
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
    ] {
        let applied = sqlx::query_scalar::<_, i64>(
            "SELECT version FROM amatl_schema_migrations WHERE version = ?",
        )
        .bind(version)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(operation)?;
        if applied.is_none() {
            sqlx::raw_sql(migration)
                .execute(&mut *transaction)
                .await
                .map_err(operation)?;
            sqlx::query(
                "INSERT INTO amatl_schema_migrations(version, name, applied_at) VALUES (?, ?, ?)",
            )
            .bind(version)
            .bind(name)
            .bind(now_unix())
            .execute(&mut *transaction)
            .await
            .map_err(operation)?;
        }
    }
    transaction.commit().await.map_err(operation)?;
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
            .document_cache_put("https://a.test", "h1", "e1", "one", 100, 1, 1_000)
            .await
            .unwrap();
        storage
            .document_cache_put("https://b.test", "h2", "e1", "two", 101, 1, 1_000)
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
}
