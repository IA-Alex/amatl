use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::{Row, SqlitePool};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use thiserror::Error;

pub const MIGRATION_VERSION: i64 = 7;
const POOL_SIZE: u32 = 4;

#[derive(Clone)]
pub struct SqliteStorage {
    pool: SqlitePool,
    path: PathBuf,
    write_lock: Arc<tokio::sync::Mutex<()>>,
    /// Advisory file lock held for the lifetime of this storage instance.
    _lock_file: Option<Arc<std::fs::File>>,
    /// Shared maintenance state updated by the background task.
    maintenance: Arc<tokio::sync::Mutex<MaintenanceState>>,
}

/// Mutable state written by the background maintenance task and read by health checks.
#[derive(Clone, Debug, Default)]
pub struct MaintenanceState {
    pub last_purge_at: Option<i64>,
    pub last_purge_rows_removed: u64,
    pub last_backup_at: Option<i64>,
    pub last_backup_integrity_ok: bool,
    pub backup_count: u32,
    pub last_fs_error: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StorageHealth {
    pub path: PathBuf,
    pub journal_mode: String,
    pub synchronous: i64,
    pub busy_timeout_ms: i64,
    pub migration_version: i64,
    pub pool_size: u32,
    /// Whether this process holds an advisory file lock on the database.
    pub lock_held: bool,
    /// Free space on the filesystem containing the database (bytes).
    pub free_space_bytes: u64,
    /// Total space on the filesystem containing the database (bytes).
    pub total_space_bytes: u64,
    /// Percentage of filesystem space used (0.0-100.0).
    pub disk_usage_percent: f64,
    /// Whether the database file is readable by this process.
    pub readable: bool,
    /// Whether the database file is writable by this process.
    pub writable: bool,
    /// Size of the main database file (bytes).
    pub file_size_bytes: u64,
    /// Size of the WAL file (bytes).
    pub wal_size_bytes: u64,
    /// Timestamp of the last successful purge cycle (Unix seconds).
    pub last_purge_at: Option<i64>,
    /// Total rows removed during the last purge cycle.
    pub last_purge_rows_removed: u64,
    /// Timestamp of the last successful automatic backup (Unix seconds).
    pub last_backup_at: Option<i64>,
    /// Whether the last automatic backup passed integrity verification.
    pub last_backup_integrity_ok: bool,
    /// Number of automatic backups currently retained.
    pub backup_count: u32,
    /// Last filesystem-level error encountered (if any).
    pub last_fs_error: Option<String>,
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

/// One recorded security rejection.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SecurityEvent {
    pub id: i64,
    /// Unix seconds.
    pub observed_at: i64,
    /// Stable event name, for example `unauthorized` or `scope_denied`.
    pub event: String,
    pub request_id: Option<String>,
    /// Authenticated identity when the caller had one; never a secret.
    pub client_id: Option<String>,
    pub path: Option<String>,
    pub client_ip: Option<String>,
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
    #[error("another process holds the database lock; only one writer is allowed")]
    LockContention,
    #[error("filesystem error: {message}")]
    Filesystem { message: String },
    #[error("disk is full or nearly full ({free_bytes} bytes free of {total_bytes})")]
    DiskFull { free_bytes: u64, total_bytes: u64 },
    #[error("permission denied accessing database file")]
    PermissionDenied,
}

/// Verify that the database file (or its parent directory) is readable and writable.
fn check_filesystem_access(path: &Path) -> Result<(), StorageError> {
    // Check parent directory permissions.
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        if parent.exists() {
            let md = std::fs::metadata(parent).map_err(|e| StorageError::Filesystem {
                message: format!("cannot stat parent directory: {e}"),
            })?;
            if md.permissions().readonly() {
                return Err(StorageError::PermissionDenied);
            }
        }
    }
    // If the file already exists, check its permissions.
    if path.exists() {
        let md = std::fs::metadata(path).map_err(|e| StorageError::Filesystem {
            message: format!("cannot stat database file: {e}"),
        })?;
        if md.permissions().readonly() {
            return Err(StorageError::PermissionDenied);
        }
    }
    Ok(())
}

/// Acquire an advisory file lock on a `.lock` sibling of the database path.
/// Returns the lock file handle that must be kept alive for the lock to persist.
fn acquire_file_lock(db_path: &Path) -> Result<std::fs::File, StorageError> {
    use fs2::FileExt;
    // Append to the full path rather than replacing the extension, so that two
    // databases sharing a stem (`amatl.db` and `amatl.sqlite3`) do not contend
    // for the same lock file.
    let lock_path = {
        let mut value = db_path.as_os_str().to_os_string();
        value.push(".lock");
        PathBuf::from(value)
    };
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&lock_path)
        .map_err(|e| StorageError::Filesystem {
            message: format!("cannot create lock file: {e}"),
        })?;
    // Try non-blocking exclusive lock.
    file.try_lock_exclusive().map_err(|e| {
        if e.kind() == std::io::ErrorKind::WouldBlock {
            StorageError::LockContention
        } else {
            StorageError::Filesystem {
                message: format!("flock failed: {e}"),
            }
        }
    })?;
    Ok(file)
}

/// Diagnose filesystem health: free space, permissions, file sizes.
fn diagnose_filesystem(path: &Path) -> (u64, u64, f64, bool, bool, u64, u64, Option<String>) {
    let mut fs_error: Option<String> = None;

    // File sizes.
    let file_size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    // SQLite always names the write-ahead log `<database path>-wal`, whatever
    // the database extension is.
    let wal_path = {
        let mut value = path.as_os_str().to_os_string();
        value.push("-wal");
        PathBuf::from(value)
    };
    let wal_size = std::fs::metadata(&wal_path).map(|m| m.len()).unwrap_or(0);

    // Permissions.
    let readable = std::fs::metadata(path).map(|_| true).unwrap_or(false);
    let writable = std::fs::OpenOptions::new().write(true).open(path).is_ok();

    // Free space on the filesystem.
    let (free_space, total_space, disk_usage) = disk_space(path).unwrap_or_else(|e| {
        fs_error = Some(format!("disk space check failed: {e}"));
        (0, 0, 0.0)
    });

    (
        free_space,
        total_space,
        disk_usage,
        readable,
        writable,
        file_size,
        wal_size,
        fs_error,
    )
}

/// Query free and total space on the filesystem containing `path`.
///
/// Both figures come from the filesystem itself (`statvfs` on Unix,
/// `GetDiskFreeSpaceEx` on Windows) rather than from the inode metadata of the
/// directory entry, which describes the directory and not the volume.
fn disk_space(path: &Path) -> Result<(u64, u64, f64), std::io::Error> {
    let target = if path.is_dir() {
        path
    } else {
        path.parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or(Path::new("."))
    };
    let total = fs2::total_space(target)?;
    let free = fs2::available_space(target)?;
    let used = total.saturating_sub(free);
    let pct = if total > 0 {
        (used as f64 / total as f64) * 100.0
    } else {
        0.0
    };
    Ok((free, total, pct))
}

/// Build a periodic timer, or `None` when the period is the documented
/// "disabled" value of zero (`tokio::time::interval` panics on a zero period).
fn interval_or_never(period_seconds: u64) -> Option<tokio::time::Interval> {
    if period_seconds == 0 {
        return None;
    }
    let mut timer = tokio::time::interval(Duration::from_secs(period_seconds));
    // The first tick completes immediately; consume it so the first real firing
    // happens one full period from now.
    timer.reset();
    Some(timer)
}

/// Await the next firing of an optional timer, or never when it is disabled.
async fn tick(timer: &mut Option<tokio::time::Interval>) {
    match timer {
        Some(timer) => {
            timer.tick().await;
        }
        None => std::future::pending().await,
    }
}

/// Filename infix of a periodic backup written by the maintenance task.
const AUTO_BACKUP_INFIX: &str = "-auto-";
/// Filename infix of the safety copy taken before a schema downgrade.
const MIGRATION_BACKUP_INFIX: &str = ".backup-";
/// Filename infix of the safety copy taken before restoring a backup.
const PRE_RESTORE_BACKUP_INFIX: &str = ".pre-restore-";

/// Why a backup file exists, which decides whether rotation may delete it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackupKind {
    /// Written periodically by the maintenance task; rotated by count.
    Auto,
    /// Written before a schema downgrade; never rotated automatically.
    Migration,
    /// Written before a restore overwrites the live database; never rotated.
    PreRestore,
}

/// A backup file discovered on disk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BackupEntry {
    pub path: PathBuf,
    pub kind: BackupKind,
    /// Unix timestamp parsed from the filename.
    pub created_at: i64,
}

/// Stem used to name every backup belonging to `db_path`.
fn database_stem(db_path: &Path) -> String {
    db_path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("amatl")
        .to_owned()
}

/// Directory holding backups for `db_path`, honoring the configured override.
fn backup_directory(db_path: &Path, configured: Option<&str>) -> PathBuf {
    configured.map(PathBuf::from).unwrap_or_else(|| {
        db_path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or(Path::new("."))
            .to_path_buf()
    })
}

/// Scan `dir` for backups of `stem`, newest first.
///
/// Recognizes all three historical naming schemes so that a single helper backs
/// rotation, counting and the operator-facing listing; previously each of those
/// carried its own filter and they disagreed about which files existed.
fn scan_backups(dir: &Path, stem: &str) -> Vec<BackupEntry> {
    let candidates = [
        (AUTO_BACKUP_INFIX, BackupKind::Auto),
        (MIGRATION_BACKUP_INFIX, BackupKind::Migration),
        (PRE_RESTORE_BACKUP_INFIX, BackupKind::PreRestore),
    ];
    let mut entries = Vec::new();
    let Ok(listing) = std::fs::read_dir(dir) else {
        return entries;
    };
    for item in listing.flatten() {
        let path = item.path();
        if !path.is_file() {
            continue;
        }
        let name = item.file_name();
        let Some(name) = name.to_str() else { continue };
        let Some(rest) = name
            .strip_prefix(stem)
            .and_then(|rest| rest.strip_suffix(".sqlite3"))
        else {
            continue;
        };
        for (infix, kind) in candidates {
            if let Some(timestamp) = rest.strip_prefix(infix) {
                if let Ok(created_at) = timestamp.parse::<i64>() {
                    entries.push(BackupEntry {
                        path,
                        kind,
                        created_at,
                    });
                }
                break;
            }
        }
    }
    entries.sort_by_key(|entry| std::cmp::Reverse(entry.created_at));
    entries
}

impl SqliteStorage {
    pub async fn open(
        path: impl AsRef<Path>,
        locking_mode: crate::config::SqliteLockingMode,
    ) -> Result<Self, StorageError> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent).map_err(|_| StorageError::Open)?;
        }

        // Verify filesystem permissions before attempting to open.
        check_filesystem_access(&path)?;

        quarantine_if_header_is_invalid(&path)?;

        // Acquire an advisory file lock when exclusive mode is requested.
        let lock_file = if locking_mode == crate::config::SqliteLockingMode::Exclusive {
            Some(acquire_file_lock(&path)?)
        } else {
            None
        };

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
            _lock_file: lock_file.map(Arc::new),
            maintenance: Arc::new(tokio::sync::Mutex::new(MaintenanceState::default())),
        })
    }

    pub async fn health(&self) -> Result<StorageHealth, StorageError> {
        let maintenance = self.maintenance.lock().await.clone();
        let (free_space, total_space, disk_usage, readable, writable, file_size, wal_size, fs_err) =
            diagnose_filesystem(&self.path);

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
            lock_held: self._lock_file.is_some(),
            free_space_bytes: free_space,
            total_space_bytes: total_space,
            disk_usage_percent: disk_usage,
            readable,
            writable,
            file_size_bytes: file_size,
            wal_size_bytes: wal_size,
            last_purge_at: maintenance.last_purge_at,
            last_purge_rows_removed: maintenance.last_purge_rows_removed,
            last_backup_at: maintenance.last_backup_at,
            last_backup_integrity_ok: maintenance.last_backup_integrity_ok,
            backup_count: maintenance.backup_count,
            last_fs_error: maintenance.last_fs_error.or(fs_err),
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

        // Consistent snapshot before a destructive schema change. Must run
        // before the write lock is taken: VACUUM cannot execute inside a
        // transaction.
        let backup_path = unused_backup_path(&self.path, MIGRATION_BACKUP_INFIX);
        self.vacuum_into(&backup_path).await?;
        tracing::info!(
            target: "amatl::storage",
            path = %self.path.display(),
            backup = %backup_path.display(),
            "database backup created before migration"
        );

        let _write_guard = self.write_lock.lock().await;
        for version in (target_version + 1..=current).rev() {
            let script = match version {
                7 => include_str!("../migrations/downgrade/0007_to_0006.sql"),
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
        locking_mode: crate::config::SqliteLockingMode,
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

        // Open with the restored file, preserving the configured locking mode.
        Self::open(path, locking_mode).await
    }

    /// Run PRAGMA integrity_check and return any error messages.
    /// Returns an empty Vec if the database is healthy.
    pub async fn integrity_check(&self) -> Result<Vec<String>, StorageError> {
        let rows: Vec<String> = sqlx::query_scalar("PRAGMA integrity_check")
            .fetch_all(&self.pool)
            .await
            .map_err(operation)?;
        // The first row is "ok" if healthy; filter it out.
        Ok(rows.into_iter().filter(|msg| msg != "ok").collect())
    }

    /// Spawn a background maintenance task that periodically prunes old data
    /// and optionally creates backups. Returns a cancel token to stop the task.
    pub fn spawn_maintenance(
        self: &Arc<Self>,
        config: &crate::config::PersistenceConfig,
        telemetry_retention_days: u32,
    ) -> tokio_util::sync::CancellationToken {
        let cancel = tokio_util::sync::CancellationToken::new();
        let cancel_clone = cancel.clone();
        let storage = Arc::clone(self);
        let history_retention = config.history_retention_days;
        let cache_retention = config.cache_retention_days;
        let doc_cache_retention = config.document_cache_retention_days;
        let audit_retention = config.audit_retention_days;
        let telemetry_retention = telemetry_retention_days;
        let purge_interval = config.purge_interval_seconds;
        let auto_backup = config.auto_backup_enabled;
        let backup_interval = config.auto_backup_interval_seconds;
        let backup_max = config.auto_backup_max_count;
        let backup_dir = config.backup_directory.clone();

        // Nothing to schedule: return an already-cancelled token rather than
        // constructing a zero-period `tokio::time::interval`, which panics.
        // `purge_interval_seconds = 0` is a documented "disabled" value.
        if purge_interval == 0 && !auto_backup {
            cancel.cancel();
            return cancel;
        }

        tokio::spawn(async move {
            // Purge and backup keep independent cadences. Folding the backup
            // into the purge tick would both silence backups whenever purging
            // is disabled and quantize them to multiples of the purge period.
            let mut purge_timer = interval_or_never(purge_interval);
            let mut backup_timer = interval_or_never(if auto_backup { backup_interval } else { 0 });

            // Seed from disk so a restart does not immediately take a fresh
            // backup and rotate healthy ones away.
            let mut last_backup_at =
                Self::latest_auto_backup_at(&storage.path, backup_dir.as_deref());

            loop {
                tokio::select! {
                    _ = cancel_clone.cancelled() => {
                        tracing::info!(target: "amatl::maintenance", "background maintenance stopped");
                        break;
                    }
                    _ = tick(&mut backup_timer) => {
                        let now = now_unix();
                        let due = last_backup_at
                            .is_none_or(|last| now.saturating_sub(last) >= backup_interval as i64);
                        if due {
                            match storage
                                .create_auto_backup(backup_dir.as_deref(), backup_max)
                                .await
                            {
                                Ok((backup_path, integrity_ok)) => {
                                    last_backup_at = Some(now);
                                    let mut state = storage.maintenance.lock().await;
                                    state.last_backup_at = Some(now);
                                    state.last_backup_integrity_ok = integrity_ok;
                                    state.backup_count = Self::count_backups(
                                        &storage.path,
                                        backup_dir.as_deref(),
                                    );
                                    tracing::info!(
                                        target: "amatl::maintenance",
                                        path = %backup_path.display(),
                                        integrity_ok,
                                        "automatic backup created"
                                    );
                                }
                                Err(e) => {
                                    tracing::error!(target: "amatl::maintenance", error = %e, "automatic backup failed");
                                    let mut state = storage.maintenance.lock().await;
                                    state.last_fs_error = Some(e.to_string());
                                }
                            }
                        }
                    }
                    _ = tick(&mut purge_timer) => {
                        let now = now_unix();
                        let mut total_removed: u64 = 0;

                        // ── Purge telemetry ──────────────────────────
                        if telemetry_retention > 0 {
                            let cutoff = now.saturating_sub(i64::from(telemetry_retention).saturating_mul(86_400));
                            match storage.telemetry_prune(cutoff).await {
                                Ok(n) => total_removed += n,
                                Err(e) => tracing::warn!(target: "amatl::maintenance", error = %e, "telemetry prune failed"),
                            }
                        }

                        // ── Purge search history ─────────────────────
                        if history_retention > 0 {
                            let cutoff = now.saturating_sub(i64::from(history_retention).saturating_mul(86_400));
                            match storage.search_history_prune(cutoff).await {
                                Ok(n) => total_removed += n,
                                Err(e) => tracing::warn!(target: "amatl::maintenance", error = %e, "history prune failed"),
                            }
                        }

                        // ── Purge provider search cache ──────────────
                        if cache_retention > 0 {
                            let cutoff = now.saturating_sub(i64::from(cache_retention).saturating_mul(86_400));
                            match storage.cache_prune_entries(cutoff).await {
                                Ok(n) => total_removed += n,
                                Err(e) => tracing::warn!(target: "amatl::maintenance", error = %e, "cache prune failed"),
                            }
                        }

                        // ── Purge document cache ─────────────────────
                        if doc_cache_retention > 0 {
                            let cutoff = now.saturating_sub(i64::from(doc_cache_retention).saturating_mul(86_400));
                            match storage.document_cache_prune_entries(cutoff).await {
                                Ok(n) => total_removed += n,
                                Err(e) => tracing::warn!(target: "amatl::maintenance", error = %e, "document cache prune failed"),
                            }
                        }

                        // ── Purge security events ────────────────────
                        if audit_retention > 0 {
                            let cutoff = now.saturating_sub(i64::from(audit_retention).saturating_mul(86_400));
                            match storage.security_events_prune(cutoff).await {
                                Ok(n) => total_removed += n,
                                Err(e) => tracing::warn!(target: "amatl::maintenance", error = %e, "audit prune failed"),
                            }
                        }

                        // ── Update maintenance state ─────────────────
                        {
                            let mut state = storage.maintenance.lock().await;
                            state.last_purge_at = Some(now);
                            state.last_purge_rows_removed = total_removed;
                        }

                        if total_removed > 0 {
                            tracing::debug!(target: "amatl::maintenance", rows_removed = total_removed, "purge cycle completed");
                        }

                        // ── Filesystem health check ─────────────────
                        let (free, total, pct, _, _, _, _, fs_err) = diagnose_filesystem(&storage.path);
                        if pct > 90.0 {
                            tracing::warn!(
                                target: "amatl::maintenance",
                                free_bytes = free,
                                total_bytes = total,
                                usage_pct = pct,
                                "disk usage is critically high"
                            );
                        }
                        if let Some(err) = fs_err {
                            let mut state = storage.maintenance.lock().await;
                            state.last_fs_error = Some(err);
                        }
                    }
                }
            }
        });

        cancel
    }

    /// Write a transactionally consistent copy of the database to `dest`.
    ///
    /// `VACUUM INTO` is a single statement executed against the live pool: it
    /// produces a fully checkpointed, self-contained database without a
    /// companion `-wal` file, and without requiring that writers be quiesced.
    /// A plain file copy cannot offer either guarantee while WAL mode is
    /// active, because the most recent commits still live in the log.
    pub(crate) async fn vacuum_into(&self, dest: &Path) -> Result<(), StorageError> {
        let Some(dest_str) = dest.to_str() else {
            return Err(StorageError::Filesystem {
                message: "backup destination path is not valid UTF-8".into(),
            });
        };
        // VACUUM cannot run inside a transaction, so this deliberately does not
        // take `write_lock`: the statement is itself atomic with respect to
        // concurrent writers.
        sqlx::query("VACUUM INTO ?")
            .bind(dest_str)
            .execute(&self.pool)
            .await
            .map_err(|error| StorageError::Filesystem {
                message: format!("VACUUM INTO failed: {error}"),
            })?;
        Ok(())
    }

    /// Trigger an on-demand backup, identical in every way to what the
    /// periodic maintenance task writes (same naming scheme, same integrity
    /// check, same rotation against `max_count`) — the only difference is
    /// *when* it runs. Exists so an operator can ask for one right before a
    /// risky change without waiting for the next scheduled interval,
    /// without adding a second backup kind or a second rotation policy to
    /// reason about.
    ///
    /// Also identical in what it *reports*: the shared [`MaintenanceState`]
    /// is updated the same way the maintenance task updates it, so
    /// `health()` (and anything built on it) reflects the manual backup
    /// immediately instead of waiting for the next automatic cycle.
    pub async fn trigger_backup(
        &self,
        backup_dir: Option<&str>,
        max_count: u32,
    ) -> Result<PathBuf, StorageError> {
        match self.create_auto_backup(backup_dir, max_count).await {
            Ok((path, integrity_ok)) => {
                let mut state = self.maintenance.lock().await;
                state.last_backup_at = Some(now_unix());
                state.last_backup_integrity_ok = integrity_ok;
                state.backup_count = Self::count_backups(&self.path, backup_dir);
                Ok(path)
            }
            Err(error) => {
                let mut state = self.maintenance.lock().await;
                state.last_fs_error = Some(error.to_string());
                Err(error)
            }
        }
    }

    /// Create an automatic backup, verify its integrity, and rotate old backups.
    async fn create_auto_backup(
        &self,
        backup_dir: Option<&str>,
        max_count: u32,
    ) -> Result<(PathBuf, bool), StorageError> {
        let dir = backup_directory(&self.path, backup_dir);
        std::fs::create_dir_all(&dir).map_err(|e| StorageError::Filesystem {
            message: format!("cannot create backup directory: {e}"),
        })?;

        let stem = database_stem(&self.path);
        let timestamp = now_unix();
        let mut backup_path = dir.join(format!("{stem}{AUTO_BACKUP_INFIX}{timestamp}.sqlite3"));
        // `VACUUM INTO` refuses to overwrite; the timestamp is second-resolution.
        for attempt in 1..1_000 {
            if !backup_path.exists() {
                break;
            }
            backup_path = dir.join(format!(
                "{stem}{AUTO_BACKUP_INFIX}{timestamp}-{attempt}.sqlite3"
            ));
        }

        self.vacuum_into(&backup_path).await?;

        let integrity_ok = Self::verify_backup_integrity(&backup_path).await;
        if !integrity_ok {
            let _ = std::fs::remove_file(&backup_path);
            // Not `Corrupt`: the live database is fine, the discarded copy is
            // not, and `Corrupt.quarantine_path` promises a file that exists.
            return Err(StorageError::Filesystem {
                message: "backup failed integrity verification and was discarded".into(),
            });
        }

        Self::rotate_backups(&dir, &stem, max_count);

        Ok((backup_path, integrity_ok))
    }

    /// Verify the integrity of a backup file by opening it and running quick_check.
    ///
    /// Opened read-only: certifying an artifact must never modify it, and
    /// setting a journal mode would both write to the backup and leave
    /// `-wal`/`-shm` siblings next to it.
    async fn verify_backup_integrity(backup_path: &Path) -> bool {
        let options = SqliteConnectOptions::new()
            .filename(backup_path)
            .create_if_missing(false)
            .read_only(true);
        let pool = match SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
        {
            Ok(p) => p,
            Err(_) => return false,
        };
        let result = sqlx::query_scalar::<_, String>("PRAGMA quick_check(1)")
            .fetch_one(&pool)
            .await;
        pool.close().await;
        matches!(result.as_deref(), Ok("ok"))
    }

    /// Rotate automatic backups, keeping only the most recent `max_count`.
    ///
    /// Only [`BackupKind::Auto`] entries are eligible: migration and
    /// pre-restore backups mark deliberate recovery points and are never
    /// rotated away by the background task.
    fn rotate_backups(dir: &Path, stem: &str, max_count: u32) {
        let backups = scan_backups(dir, stem);
        for entry in backups
            .iter()
            .filter(|entry| entry.kind == BackupKind::Auto)
            .skip(max_count as usize)
        {
            let _ = std::fs::remove_file(&entry.path);
            tracing::debug!(target: "amatl::maintenance", path = %entry.path.display(), "rotated old backup");
        }
    }

    /// Count current automatic backups.
    fn count_backups(db_path: &Path, backup_dir: Option<&str>) -> u32 {
        let dir = backup_directory(db_path, backup_dir);
        scan_backups(&dir, &database_stem(db_path))
            .iter()
            .filter(|entry| entry.kind == BackupKind::Auto)
            .count()
            .try_into()
            .unwrap_or(u32::MAX)
    }

    /// Timestamp of the most recent automatic backup on disk, if any.
    ///
    /// Used to seed the maintenance task so a restart does not trigger a fresh
    /// backup on its first tick (which would rotate healthy ones away).
    fn latest_auto_backup_at(db_path: &Path, backup_dir: Option<&str>) -> Option<i64> {
        let dir = backup_directory(db_path, backup_dir);
        scan_backups(&dir, &database_stem(db_path))
            .into_iter()
            .find(|entry| entry.kind == BackupKind::Auto)
            .map(|entry| entry.created_at)
    }

    /// List available backup files for a database path, newest first.
    ///
    /// Covers all three naming schemes, so automatic backups are visible to
    /// `amatl db backups` and selectable by `amatl db restore`.
    pub fn list_backups(
        db_path: &Path,
        backup_dir: Option<&str>,
    ) -> Result<Vec<PathBuf>, StorageError> {
        let stem = database_stem(db_path);
        let mut directories = vec![backup_directory(db_path, None)];
        if let Some(configured) = backup_dir.map(PathBuf::from) {
            if !directories.contains(&configured) {
                directories.push(configured);
            }
        }
        let mut entries: Vec<BackupEntry> = directories
            .iter()
            .flat_map(|dir| scan_backups(dir, &stem))
            .collect();
        entries.sort_by_key(|entry| std::cmp::Reverse(entry.created_at));
        Ok(entries.into_iter().map(|entry| entry.path).collect())
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

    /// Prune provider search cache entries older than `before` (Unix seconds).
    pub(crate) async fn cache_prune_entries(&self, before: i64) -> Result<u64, StorageError> {
        let _write_guard = self.write_lock.lock().await;
        Ok(
            sqlx::query("DELETE FROM provider_search_cache WHERE created_at < ?")
                .bind(before)
                .execute(&self.pool)
                .await
                .map_err(operation)?
                .rows_affected(),
        )
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

    /// Prune document cache entries older than `before` (Unix seconds).
    pub(crate) async fn document_cache_prune_entries(
        &self,
        before: i64,
    ) -> Result<u64, StorageError> {
        let _write_guard = self.write_lock.lock().await;
        Ok(
            sqlx::query("DELETE FROM document_cache WHERE created_at < ?")
                .bind(before)
                .execute(&self.pool)
                .await
                .map_err(operation)?
                .rows_affected(),
        )
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

    /// Prune search history entries older than `before` (Unix seconds).
    pub async fn search_history_prune(&self, before: i64) -> Result<u64, StorageError> {
        let _write_guard = self.write_lock.lock().await;
        Ok(
            sqlx::query("DELETE FROM search_history WHERE created_at < ?")
                .bind(before)
                .execute(&self.pool)
                .await
                .map_err(operation)?
                .rows_affected(),
        )
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

    // ── Security audit trail ────────────────────────────────────────────

    /// Append one security event and prune anything past the retention window.
    pub async fn security_event_insert(
        &self,
        event: &SecurityEvent,
        retention_days: u32,
    ) -> Result<(), StorageError> {
        let _write_guard = self.write_lock.lock().await;
        sqlx::query(
            "INSERT INTO security_events
               (observed_at, event, request_id, client_id, path, client_ip)
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(event.observed_at)
        .bind(&event.event)
        .bind(&event.request_id)
        .bind(&event.client_id)
        .bind(&event.path)
        .bind(&event.client_ip)
        .execute(&self.pool)
        .await
        .map_err(operation)?;
        let cutoff = event
            .observed_at
            .saturating_sub(i64::from(retention_days).saturating_mul(86_400));
        sqlx::query("DELETE FROM security_events WHERE observed_at < ?")
            .bind(cutoff)
            .execute(&self.pool)
            .await
            .map_err(operation)?;
        Ok(())
    }

    /// Recorded events, newest first.
    pub async fn security_events(
        &self,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<SecurityEvent>, StorageError> {
        let rows = sqlx::query(
            "SELECT id, observed_at, event, request_id, client_id, path, client_ip
             FROM security_events
             ORDER BY observed_at DESC, id DESC
             LIMIT ? OFFSET ?",
        )
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await
        .map_err(operation)?;
        Ok(rows
            .into_iter()
            .map(|row| SecurityEvent {
                id: row.get("id"),
                observed_at: row.get("observed_at"),
                event: row.get("event"),
                request_id: row.get("request_id"),
                client_id: row.get("client_id"),
                path: row.get("path"),
                client_ip: row.get("client_ip"),
            })
            .collect())
    }

    /// Count recorded events.
    pub async fn security_event_count(&self) -> Result<i64, StorageError> {
        sqlx::query_scalar("SELECT COUNT(*) FROM security_events")
            .fetch_one(&self.pool)
            .await
            .map_err(operation)
    }

    /// Prune security events older than `before` (Unix seconds).
    pub async fn security_events_prune(&self, before: i64) -> Result<u64, StorageError> {
        let _write_guard = self.write_lock.lock().await;
        Ok(
            sqlx::query("DELETE FROM security_events WHERE observed_at < ?")
                .bind(before)
                .execute(&self.pool)
                .await
                .map_err(operation)?
                .rows_affected(),
        )
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
            Self::Open
            | Self::Operation
            | Self::IncompatibleVersion { .. }
            | Self::LockContention
            | Self::Filesystem { .. }
            | Self::DiskFull { .. }
            | Self::PermissionDenied => None,
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
    // Read the version outside a transaction: the pre-migration backup below
    // uses `VACUUM INTO`, which cannot run inside one.
    let db_version: i64 = sqlx::query_scalar("PRAGMA user_version")
        .fetch_one(&pool)
        .await
        .map_err(operation)?;
    if db_version > MIGRATION_VERSION {
        return Err(StorageError::IncompatibleVersion {
            db_version,
            code_version: MIGRATION_VERSION,
        });
    }

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
        (
            7_i64,
            "security_events",
            include_str!("../migrations/0007_security_events.sql"),
        ),
    ]
    .into_iter()
    .filter(|(version, _, _)| *version > db_version)
    .collect();

    // Create a consistent backup before applying any pending migrations.
    if !pending.is_empty() {
        backup_database(&pool, &db_path).await?;
    }

    let mut transaction = pool.begin().await.map_err(operation)?;

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

/// Pick a backup path of `kind` that does not yet exist.
///
/// `VACUUM INTO` refuses to overwrite, and the timestamp only has second
/// resolution, so two backups taken in the same second (a migration followed
/// immediately by a downgrade, for instance) would otherwise collide.
fn unused_backup_path(db_path: &Path, infix: &str) -> PathBuf {
    let dir = backup_directory(db_path, None);
    let stem = database_stem(db_path);
    let timestamp = now_unix();
    for attempt in 0..1_000 {
        let candidate = if attempt == 0 {
            dir.join(format!("{stem}{infix}{timestamp}.sqlite3"))
        } else {
            dir.join(format!("{stem}{infix}{timestamp}-{attempt}.sqlite3"))
        };
        if !candidate.exists() {
            return candidate;
        }
    }
    dir.join(format!(
        "{stem}{infix}{timestamp}-{}.sqlite3",
        std::process::id()
    ))
}

/// Create a consistent, timestamped backup of the database before migration.
async fn backup_database(pool: &SqlitePool, path: &Path) -> Result<(), StorageError> {
    let backup_path = unused_backup_path(path, MIGRATION_BACKUP_INFIX);
    let Some(destination) = backup_path.to_str() else {
        return Err(StorageError::Filesystem {
            message: "backup destination path is not valid UTF-8".into(),
        });
    };
    // `VACUUM INTO` rather than a file copy: in WAL mode the newest commits
    // live in the log, so copying only the main file can silently drop them
    // right before a destructive schema change.
    sqlx::query("VACUUM INTO ?")
        .bind(destination)
        .execute(pool)
        .await
        .map_err(|error| StorageError::Filesystem {
            message: format!("VACUUM INTO failed: {error}"),
        })?;
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

    #[test]
    fn disk_space_reports_the_real_filesystem() {
        let (free, total, pct) = disk_space(&std::env::temp_dir()).expect("statvfs");
        assert!(total > 0, "total space must come from the filesystem");
        assert!(
            free <= total,
            "free ({free}) must not exceed total ({total})"
        );
        assert!(
            (0.0..=100.0).contains(&pct),
            "usage percent out of range: {pct}"
        );
    }

    #[tokio::test]
    async fn auto_backup_is_consistent_restorable_and_leaves_no_wal() {
        let path = path("auto-backup");
        let storage = SqliteStorage::open(&path, crate::config::SqliteLockingMode::Normal)
            .await
            .unwrap();
        storage
            .search_history_insert("backup probe", "backup probe", None, 1, 1, 0, "cli")
            .await
            .unwrap();

        let (backup_path, integrity_ok) = storage.create_auto_backup(None, 5).await.unwrap();
        assert!(integrity_ok, "fresh backup must verify");

        // `VACUUM INTO` yields a fully checkpointed file, and verification is
        // read-only, so no journal siblings may be left next to the backup.
        for suffix in ["-wal", "-shm"] {
            let mut sibling = backup_path.as_os_str().to_os_string();
            sibling.push(suffix);
            assert!(
                !PathBuf::from(&sibling).exists(),
                "backup must not carry {suffix}"
            );
        }

        // The committed row must be present in the backup itself.
        let restored_path = path.with_extension("restored.sqlite3");
        let restored = SqliteStorage::restore_from_backup(
            &restored_path,
            &backup_path,
            crate::config::SqliteLockingMode::Normal,
        )
        .await
        .unwrap();
        assert_eq!(
            restored.search_history_count().await.unwrap(),
            1,
            "backup lost a committed transaction"
        );
    }

    #[tokio::test]
    async fn trigger_backup_updates_the_shared_maintenance_state() {
        // Regression: an on-demand backup claimed to be "identical in every
        // way" to the periodic task but never touched `MaintenanceState`, so
        // `health()` kept reporting the previous automatic cycle until the
        // next one ran.
        let path = path("trigger-backup");
        let storage = SqliteStorage::open(&path, crate::config::SqliteLockingMode::Normal)
            .await
            .unwrap();
        assert_eq!(storage.health().await.unwrap().last_backup_at, None);

        let backup_path = storage.trigger_backup(None, 5).await.unwrap();
        let health = storage.health().await.unwrap();
        assert!(
            health.last_backup_at.is_some(),
            "manual backup must update last_backup_at"
        );
        assert!(health.last_backup_integrity_ok, "fresh backup must verify");
        assert_eq!(health.backup_count, 1);

        // A second manual backup rotates to the configured maximum, and the
        // reported count tracks it. Backups are named at second resolution, so
        // space the triggers out to keep the filenames distinct.
        tokio::time::sleep(Duration::from_secs(1)).await;
        storage.trigger_backup(None, 2).await.unwrap();
        tokio::time::sleep(Duration::from_secs(1)).await;
        storage.trigger_backup(None, 2).await.unwrap();
        let health = storage.health().await.unwrap();
        assert_eq!(health.backup_count, 2, "rotation must respect max_count");

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(backup_path);
        for entry in scan_backups(path.parent().unwrap(), &database_stem(&path)) {
            let _ = std::fs::remove_file(entry.path);
        }
    }

    #[tokio::test]
    async fn rotation_keeps_recent_auto_backups_and_spares_other_kinds() {
        let path = path("rotation");
        let storage = SqliteStorage::open(&path, crate::config::SqliteLockingMode::Normal)
            .await
            .unwrap();
        let dir = path.parent().unwrap().to_path_buf();
        let stem = database_stem(&path);

        // Three automatic backups plus one of each protected kind.
        for offset in 1..=3 {
            std::fs::copy(
                &path,
                dir.join(format!("{stem}{AUTO_BACKUP_INFIX}{offset}.sqlite3")),
            )
            .unwrap();
        }
        std::fs::copy(
            &path,
            dir.join(format!("{stem}{MIGRATION_BACKUP_INFIX}1.sqlite3")),
        )
        .unwrap();
        std::fs::copy(
            &path,
            dir.join(format!("{stem}{PRE_RESTORE_BACKUP_INFIX}1.sqlite3")),
        )
        .unwrap();

        SqliteStorage::rotate_backups(&dir, &stem, 2);

        let remaining = scan_backups(&dir, &stem);
        let auto: Vec<i64> = remaining
            .iter()
            .filter(|entry| entry.kind == BackupKind::Auto)
            .map(|entry| entry.created_at)
            .collect();
        assert_eq!(auto, vec![3, 2], "rotation must keep the newest two");
        assert!(
            remaining
                .iter()
                .any(|entry| entry.kind == BackupKind::Migration),
            "migration backups are never rotated"
        );
        assert!(
            remaining
                .iter()
                .any(|entry| entry.kind == BackupKind::PreRestore),
            "pre-restore backups are never rotated"
        );

        // Automatic backups must also be visible to the operator listing.
        let listed = SqliteStorage::list_backups(&path, None).unwrap();
        assert!(
            listed
                .iter()
                .any(|entry| entry.to_string_lossy().contains(AUTO_BACKUP_INFIX)),
            "db backups must list automatic backups"
        );

        drop(storage);
    }

    #[tokio::test]
    async fn configures_wal_busy_timeout_and_versioned_migration() {
        let path = path("health");
        let storage = SqliteStorage::open(&path, crate::config::SqliteLockingMode::Normal)
            .await
            .unwrap();
        let health = storage.health().await.unwrap();
        assert_eq!(health.journal_mode.to_ascii_lowercase(), "wal");
        assert_eq!(health.synchronous, 1);
        assert_eq!(health.busy_timeout_ms, 5_000);
        assert_eq!(health.migration_version, MIGRATION_VERSION);
        assert_eq!(health.pool_size, POOL_SIZE);
        storage.pool.close().await;

        let reopened = SqliteStorage::open(&path, crate::config::SqliteLockingMode::Normal)
            .await
            .unwrap();
        assert_eq!(
            reopened.health().await.unwrap().migration_version,
            MIGRATION_VERSION
        );
        reopened.pool.close().await;
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn saved_document_crud_round_trips() {
        let storage =
            SqliteStorage::open(path("saved-crud"), crate::config::SqliteLockingMode::Normal)
                .await
                .unwrap();
        let id = storage
            .saved_document_put(
                "https://example.com/doc",
                Some("Example"),
                Some("A snippet"),
                "hash-1",
                "extractor-v1",
                "payload-bytes",
                Some("rust"),
                "tag-a,tag-b",
            )
            .await
            .unwrap();
        assert!(id > 0);

        // Read back the persisted document.
        let loaded = storage
            .saved_document_get("https://example.com/doc", "hash-1", "extractor-v1")
            .await
            .unwrap()
            .expect("saved document must be readable");
        assert_eq!(loaded.canonical_url, "https://example.com/doc");
        assert_eq!(loaded.title.as_deref(), Some("Example"));
        assert_eq!(loaded.snippet.as_deref(), Some("A snippet"));
        assert_eq!(loaded.payload, "payload-bytes");
        assert_eq!(loaded.source_query.as_deref(), Some("rust"));
        assert_eq!(loaded.tags, "tag-a,tag-b");

        // Upsert on the same (url, hash, extractor) updates instead of duplicating.
        storage
            .saved_document_put(
                "https://example.com/doc",
                Some("Example Updated"),
                Some("New snippet"),
                "hash-1",
                "extractor-v1",
                "new-payload",
                Some("rust"),
                "tag-a",
            )
            .await
            .unwrap();
        let list = storage.saved_document_list(10, 0).await.unwrap();
        assert_eq!(list.len(), 1, "upsert must not create a duplicate row");
        assert_eq!(list[0].title.as_deref(), Some("Example Updated"));
        assert_eq!(list[0].payload, "new-payload");

        // Delete removes the row.
        assert!(storage.saved_document_delete(id).await.unwrap());
        assert!(storage
            .saved_document_get("https://example.com/doc", "hash-1", "extractor-v1")
            .await
            .unwrap()
            .is_none());
        storage.pool.close().await;
        let _ = std::fs::remove_file(path("saved-crud"));
    }

    #[tokio::test]
    async fn cache_enforces_ttl_and_lru_entry_limit() {
        let storage = SqliteStorage::open(
            path("cache-policy"),
            crate::config::SqliteLockingMode::Normal,
        )
        .await
        .unwrap();
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
        let storage = SqliteStorage::open(
            path("cache-quota"),
            crate::config::SqliteLockingMode::Normal,
        )
        .await
        .unwrap();
        storage
            .cache_put("p", "v1", "q", "{}", "four", 100, 100, 10, 3)
            .await
            .unwrap();
        assert_eq!(storage.cache_stats().await.unwrap(), CacheStats::default());
    }

    #[tokio::test]
    async fn document_cache_is_versioned_and_enforces_lru_quota() {
        let storage = SqliteStorage::open(
            path("document-cache-policy"),
            crate::config::SqliteLockingMode::Normal,
        )
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
        let storage = SqliteStorage::open(
            path("document-cache-ttl"),
            crate::config::SqliteLockingMode::Normal,
        )
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
