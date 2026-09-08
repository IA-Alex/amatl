use amatl_core::{
    parse_query, Budget, CachedProvider, Category, InMemoryTelemetry, MockProvider, Provider,
    ProviderItem, ProviderSearchCache, ProviderSearchCachePolicy, Rank, SearchOrchestrator,
    SearchStatus, SqliteStorage, StorageError, TelemetryObservation, TelemetryOutcome,
};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

static TEMP_ID: AtomicUsize = AtomicUsize::new(0);

fn path(name: &str) -> PathBuf {
    let id = TEMP_ID.fetch_add(1, Ordering::SeqCst);
    std::env::temp_dir().join(format!(
        "amatl-phase3-{name}-{}-{id}.sqlite3",
        std::process::id()
    ))
}

fn item(url: &str) -> ProviderItem {
    ProviderItem {
        title: Some("rust result".into()),
        url: url.into(),
        provider_rank: Some(Rank::new(1).unwrap()),
        snippet: None,
        result_type: None,
        published_at: None,
        author: None,
        language: None,
        file_type: None,
        thumbnail: None,
        metadata: BTreeMap::new(),
    }
}

fn enabled_cache(storage: SqliteStorage) -> ProviderSearchCache {
    ProviderSearchCache::new(
        storage,
        ProviderSearchCachePolicy {
            enabled: true,
            ttl_seconds: 300,
            max_entries: 100,
            max_bytes: 1_000_000,
        },
    )
}

#[tokio::test]
async fn provider_cache_runs_before_pipeline_and_avoids_second_adapter_call() {
    let storage = SqliteStorage::open(path("hit"), amatl_core::config::SqliteLockingMode::Normal)
        .await
        .unwrap();
    let mock = Arc::new(MockProvider::success(
        "p",
        vec![item("https://example.com/?utm_source=provider")],
    ));
    let cached: Arc<dyn Provider> = Arc::new(CachedProvider::new(
        mock.clone(),
        enabled_cache(storage),
        "adapter-v1",
        true,
    ));
    for _ in 0..2 {
        let response = SearchOrchestrator::new(Budget::new(1, 1_000), 100)
            .search(parse_query("rust".into()).unwrap(), vec![cached.clone()])
            .await;
        assert_eq!(response.status, SearchStatus::Success);
        assert_eq!(
            response.results[0].canonical_url.0.as_str(),
            "https://example.com/"
        );
    }
    assert_eq!(mock.attempts(), 1);
}

#[tokio::test]
async fn missing_storage_rights_bypasses_cache_even_when_globally_enabled() {
    let storage = SqliteStorage::open(
        path("rights"),
        amatl_core::config::SqliteLockingMode::Normal,
    )
    .await
    .unwrap();
    let cache = enabled_cache(storage);
    let mock = Arc::new(MockProvider::success(
        "p",
        vec![item("https://example.com/")],
    ));
    let cached: Arc<dyn Provider> = Arc::new(CachedProvider::new(
        mock.clone(),
        cache.clone(),
        "adapter-v1",
        false,
    ));
    for _ in 0..2 {
        SearchOrchestrator::new(Budget::new(1, 1_000), 100)
            .search(parse_query("rust".into()).unwrap(), vec![cached.clone()])
            .await;
    }
    assert_eq!(mock.attempts(), 2);
    assert_eq!(cache.stats().await.entries, 0);
}

#[tokio::test]
async fn search_records_live_provider_health_for_later_routing() {
    let telemetry = InMemoryTelemetry::new();
    let providers: Vec<Arc<dyn Provider>> = vec![Arc::new(MockProvider::success(
        "p",
        vec![item("https://one.example/"), item("https://two.example/")],
    ))];
    let response = SearchOrchestrator::new(Budget::new(1, 1_000), 100)
        .with_telemetry(telemetry.clone())
        .search(parse_query("rust".into()).unwrap(), providers)
        .await;
    assert_eq!(response.status, SearchStatus::Success);
    let snapshot = telemetry.snapshot_global("p", amatl_core::telemetry::now_unix());
    assert_eq!(snapshot.sample, 1);
    assert_eq!(snapshot.average_unique_results, 2.0);
    assert_eq!(snapshot.top_k_contribution, 1.0);
    assert_eq!(snapshot.diversity, 1.0);
}

#[tokio::test]
async fn optional_telemetry_persistence_restores_window_but_memory_remains_authoritative() {
    let storage = SqliteStorage::open(
        path("telemetry"),
        amatl_core::config::SqliteLockingMode::Normal,
    )
    .await
    .unwrap();
    let now = amatl_core::telemetry::now_unix();
    let telemetry = InMemoryTelemetry::with_optional_storage(Some(storage.clone())).await;
    telemetry
        .record(TelemetryObservation {
            observed_at: now,
            provider: "p".into(),
            category: Category::Technical,
            outcome: TelemetryOutcome::Success,
            latency_ms: 25,
            total_results: 3,
            unique_results: 3,
            duplicate_ratio: 0.0,
            top_k_contribution: 1.0,
            diversity: 1.0,
            cost_units: 5,
            request_id: None,
        })
        .await;
    drop(telemetry);
    let restored = InMemoryTelemetry::with_optional_storage(Some(storage)).await;
    assert_eq!(restored.snapshot_global("p", now).sample, 1);
    assert!(restored.status().persistence_enabled);
}

#[tokio::test]
async fn corrupt_database_is_quarantined_without_overwrite() {
    let database = path("corrupt");
    std::fs::write(&database, b"not a sqlite database").unwrap();
    let error =
        match SqliteStorage::open(&database, amatl_core::config::SqliteLockingMode::Normal).await {
            Ok(_) => panic!("corrupt database must not open"),
            Err(error) => error,
        };
    let StorageError::Corrupt { quarantine_path } = error else {
        panic!("corruption must be quarantined");
    };
    assert!(!database.exists());
    assert!(quarantine_path.exists());
    assert_eq!(
        std::fs::read(&quarantine_path).unwrap(),
        b"not a sqlite database"
    );
    std::fs::remove_file(quarantine_path).unwrap();
}

/// Read a single line from `reader`, failing if `deadline` passes first.
///
/// Runs the blocking read on a helper thread so the timeout is real even when
/// the child never writes anything.
fn read_line_before(
    mut reader: std::process::ChildStdout,
    deadline: std::time::Duration,
) -> Result<String, String> {
    use std::io::BufRead;
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut line = String::new();
        let result = std::io::BufReader::new(&mut reader)
            .read_line(&mut line)
            .map(|n| (n, line));
        let _ = tx.send(result);
    });
    match rx.recv_timeout(deadline) {
        Ok(Ok((0, _))) => Err("child closed stdout without printing a line".into()),
        Ok(Ok((_, line))) => Ok(line),
        Ok(Err(e)) => Err(format!("reading child stdout failed: {e}")),
        Err(_) => Err(format!("child printed nothing within {deadline:?}")),
    }
}

/// Wait for `child` to exit, failing if `deadline` passes first.
fn wait_before(
    child: &mut std::process::Child,
    deadline: std::time::Duration,
) -> Result<std::process::ExitStatus, String> {
    let step = std::time::Duration::from_millis(20);
    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) => {}
            Err(e) => return Err(format!("try_wait failed: {e}")),
        }
        if start.elapsed() >= deadline {
            return Err(format!("child still running after {deadline:?}"));
        }
        std::thread::sleep(step);
    }
}

/// Spawn the `lock_probe` auxiliary binary against `database`.
fn spawn_probe(database: &std::path::Path) -> std::process::Child {
    std::process::Command::new(env!("CARGO_BIN_EXE_lock_probe"))
        .arg(database)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn lock_probe")
}

/// The background maintenance task must not outlive the service that spawned
/// it. It holds a clone of the storage handle, so a leaked task keeps both the
/// connection pool and the advisory file lock alive; under
/// `locking_mode = "exclusive"` no other process could then open the database.
#[tokio::test]
async fn dropping_the_service_releases_the_exclusive_database_lock() {
    use amatl_core::config::SqliteLockingMode;
    use amatl_core::{AmatlService, Config};

    let timeout = std::time::Duration::from_secs(5);
    let database = path("maintenance-lock");
    let mut config = Config::default();
    config.persistence.enabled = true;
    config.persistence.path = database.to_string_lossy().into_owned();
    config.persistence.locking_mode = SqliteLockingMode::Exclusive;
    // Long enough that the task is certainly still waiting on its first tick.
    config.persistence.purge_interval_seconds = 3_600;
    config.validate().expect("config is valid");

    // Sanity-check the auxiliary binary against a free database (nothing has
    // opened it yet): a fresh process must acquire the lock, announce "LOCKED",
    // and then block on stdin rather than exit. Windows scopes LockFileEx locks
    // to the process, not the handle, so a real second process is the only
    // check that means the same thing on all three platforms -- a second
    // in-process `SqliteStorage::open` would never contend there.
    let mut probe = spawn_probe(&database);
    match read_line_before(probe.stdout.take().unwrap(), timeout) {
        Ok(line) => assert_eq!(
            line.trim(),
            "LOCKED",
            "a probe against a free database must acquire the lock"
        ),
        Err(e) => panic!("probe never reported LOCKED: {e}"),
    }
    // It must now be blocked on stdin, not exited.
    assert!(
        probe.try_wait().expect("try_wait").is_none(),
        "probe holding the lock must block on stdin, not exit"
    );
    // Closing stdin lets it exit cleanly and release the lock.
    drop(probe.stdin.take());
    let status = wait_before(&mut probe, timeout).expect("probe should exit after stdin closes");
    assert_eq!(
        status.code(),
        Some(0),
        "probe should exit 0 after releasing"
    );
    // Wait for the OS to actually drop the released lock before the service
    // tries to take it (belt and suspenders -- `wait` already reaped the child).
    for _ in 0..50 {
        match SqliteStorage::open(&database, SqliteLockingMode::Exclusive).await {
            Ok(handle) => {
                drop(handle);
                break;
            }
            Err(StorageError::LockContention) => {
                tokio::time::sleep(std::time::Duration::from_millis(20)).await
            }
            Err(e) => panic!("unexpected error opening a released database: {e:?}"),
        }
    }

    // Now bring the service up; it holds the exclusive lock for as long as it
    // lives.
    let service = AmatlService::new(config, true).await;

    // Real cross-process contention: an independent process trying to open the
    // same database while the service holds the lock must see LockContention
    // and exit 42. This is the assertion that was red on windows-latest when
    // the test opened the storage twice in one process.
    let mut contender = spawn_probe(&database);
    let status = wait_before(&mut contender, timeout)
        .expect("a second process must fail fast, not hang, while the lock is held");
    if status.code() != Some(42) {
        let mut stderr = String::new();
        if let Some(mut e) = contender.stderr.take() {
            use std::io::Read;
            let _ = e.read_to_string(&mut stderr);
        }
        panic!(
            "contender exit code was {:?} (expected 42 = LockContention); stderr: {stderr}",
            status.code()
        );
    }

    drop(service);
    // Cancellation is observed by the task asynchronously; give it a moment to
    // unwind and release the pool. This poll only checks that the parent no
    // longer holds the lock, not contention, so it can stay in-process.
    for _ in 0..50 {
        if SqliteStorage::open(&database, SqliteLockingMode::Exclusive)
            .await
            .is_ok()
        {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    panic!("the advisory lock was never released after dropping the service");
}
