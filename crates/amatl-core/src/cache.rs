use crate::model::{ProviderError, ProviderResult, SearchPlan};
use crate::providers::{Provider, ProviderAvailability, ProviderContext};
use crate::storage::{CacheStats, SqliteStorage};
use crate::telemetry::now_unix;
use async_trait::async_trait;
use std::collections::BTreeMap;
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderSearchCachePolicy {
    pub enabled: bool,
    pub ttl_seconds: u64,
    pub max_entries: u64,
    pub max_bytes: u64,
}

impl Default for ProviderSearchCachePolicy {
    fn default() -> Self {
        Self {
            enabled: false,
            ttl_seconds: 300,
            max_entries: 10_000,
            max_bytes: 268_435_456,
        }
    }
}

#[derive(Clone)]
pub struct ProviderSearchCache {
    storage: SqliteStorage,
    policy: ProviderSearchCachePolicy,
}

impl ProviderSearchCache {
    pub fn new(storage: SqliteStorage, policy: ProviderSearchCachePolicy) -> Self {
        Self { storage, policy }
    }

    pub async fn get(
        &self,
        provider: &str,
        adapter_version: &str,
        plan: &SearchPlan,
    ) -> Option<ProviderResult> {
        if !self.policy.enabled {
            return None;
        }
        let filters = structured_filters(plan);
        self.storage
            .cache_get(
                provider,
                adapter_version,
                &plan.query.normalized_query,
                &filters,
                now_unix(),
                self.policy.ttl_seconds,
            )
            .await
            .ok()
            .flatten()
            .and_then(|payload| serde_json::from_str(&payload).ok())
    }

    pub async fn put(
        &self,
        provider: &str,
        adapter_version: &str,
        plan: &SearchPlan,
        result: &ProviderResult,
    ) {
        if !self.policy.enabled {
            return;
        }
        let Ok(payload) = serde_json::to_string(result) else {
            return;
        };
        let _ = self
            .storage
            .cache_put(
                provider,
                adapter_version,
                &plan.query.normalized_query,
                &structured_filters(plan),
                &payload,
                now_unix(),
                self.policy.ttl_seconds,
                self.policy.max_entries,
                self.policy.max_bytes,
            )
            .await;
    }

    pub async fn stats(&self) -> CacheStats {
        self.storage.cache_stats().await.unwrap_or_default()
    }

    pub async fn purge(&self) -> u64 {
        self.storage.cache_purge().await.unwrap_or(0)
    }
}

pub struct CachedProvider {
    inner: Arc<dyn Provider>,
    cache: ProviderSearchCache,
    adapter_version: String,
    storage_rights: bool,
}

impl CachedProvider {
    pub fn new(
        inner: Arc<dyn Provider>,
        cache: ProviderSearchCache,
        adapter_version: impl Into<String>,
        storage_rights: bool,
    ) -> Self {
        Self {
            inner,
            cache,
            adapter_version: adapter_version.into(),
            storage_rights,
        }
    }
}

#[async_trait]
impl Provider for CachedProvider {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn capabilities(&self) -> crate::ProviderCapabilities {
        self.inner.capabilities()
    }

    fn availability(&self) -> ProviderAvailability {
        self.inner.availability()
    }

    async fn search(
        &self,
        plan: &SearchPlan,
        context: &ProviderContext,
    ) -> Result<ProviderResult, ProviderError> {
        if self.storage_rights {
            if let Some(result) = self
                .cache
                .get(self.name(), &self.adapter_version, plan)
                .await
            {
                return Ok(result);
            }
        }
        let result = self.inner.search(plan, context).await?;
        if self.storage_rights {
            self.cache
                .put(self.name(), &self.adapter_version, plan, &result)
                .await;
        }
        Ok(result)
    }
}

fn structured_filters(plan: &SearchPlan) -> String {
    let query = &plan.query;
    let filters = BTreeMap::from([
        ("date_from", query.date_from.clone().unwrap_or_default()),
        ("date_to", query.date_to.clone().unwrap_or_default()),
        ("domains", query.domains.join(",")),
        ("excluded_domains", query.excluded_domains.join(",")),
        ("excluded_terms", query.excluded_terms.join(",")),
        ("file_types", query.file_types.join(",")),
        ("language", query.language.clone().unwrap_or_default()),
        ("quoted_terms", query.quoted_terms.join(",")),
        ("region", query.region.clone().unwrap_or_default()),
    ]);
    serde_json::to_string(&filters).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::planning::build_search_plan;
    use crate::router::RoutingRecommendation;
    use crate::{classify, parse_query, Budget, MockProvider, ProviderItem, Rank};
    use std::sync::atomic::{AtomicUsize, Ordering};

    async fn temporary_storage() -> SqliteStorage {
        let id = TEMP_ID.fetch_add(1, Ordering::SeqCst);
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        SqliteStorage::open(std::env::temp_dir().join(format!(
            "amatl-cache-{}-{nonce}-{id}.sqlite3",
            std::process::id()
        )))
        .await
        .unwrap()
    }
    static TEMP_ID: AtomicUsize = AtomicUsize::new(0);

    fn plan() -> SearchPlan {
        let query = parse_query("rust lang:es".into()).unwrap();
        build_search_plan(
            query.clone(),
            classify(&query),
            RoutingRecommendation {
                selected_providers: vec!["p".into()],
                provider_budget_requests: BTreeMap::from([("p".into(), 1)]),
                debug_reasons: vec![],
            },
            &mut Budget::new(1, 1_000),
        )
    }

    async fn result() -> ProviderResult {
        MockProvider::success(
            "p",
            vec![ProviderItem {
                title: Some("rust".into()),
                url: "https://example.com".into(),
                provider_rank: Some(Rank::FIRST),
                snippet: None,
                result_type: None,
                published_at: None,
                author: None,
                language: None,
                file_type: None,
                thumbnail: None,
                metadata: BTreeMap::new(),
            }],
        )
        .search(&plan(), &ProviderContext { timeout_ms: 100 })
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn key_includes_adapter_version_and_structured_filters() {
        let cache = ProviderSearchCache::new(
            temporary_storage().await,
            ProviderSearchCachePolicy {
                enabled: true,
                ..Default::default()
            },
        );
        let value = result().await;
        cache.put("p", "v1", &plan(), &value).await;
        assert!(cache.get("p", "v1", &plan()).await.is_some());
        assert!(cache.get("p", "v2", &plan()).await.is_none());
    }

    #[tokio::test]
    async fn disabled_cache_never_reads_or_writes() {
        let cache = ProviderSearchCache::new(
            temporary_storage().await,
            ProviderSearchCachePolicy::default(),
        );
        cache.put("p", "v1", &plan(), &result().await).await;
        assert!(cache.get("p", "v1", &plan()).await.is_none());
        assert_eq!(cache.stats().await.entries, 0);
    }
}
