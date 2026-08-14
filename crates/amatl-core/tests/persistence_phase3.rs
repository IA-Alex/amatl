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
    let storage = SqliteStorage::open(path("hit")).await.unwrap();
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
    let storage = SqliteStorage::open(path("rights")).await.unwrap();
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
    let storage = SqliteStorage::open(path("telemetry")).await.unwrap();
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
    let error = match SqliteStorage::open(&database).await {
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
