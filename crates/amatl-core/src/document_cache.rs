use crate::model::Document;
use crate::storage::{CacheStats, SqliteStorage};

#[derive(Clone, Debug)]
pub struct DocumentCachePolicy {
    pub enabled: bool,
    pub ttl_seconds: u64,
    pub max_entries: u64,
    pub max_bytes: u64,
    pub store_content: bool,
}

#[derive(Clone)]
pub struct DocumentCache {
    storage: SqliteStorage,
    policy: DocumentCachePolicy,
}

impl DocumentCache {
    pub fn new(storage: SqliteStorage, policy: DocumentCachePolicy) -> Self {
        Self { storage, policy }
    }

    pub async fn get(
        &self,
        canonical_url: &str,
        content_hash: &str,
        extractor_version: &str,
    ) -> Option<Document> {
        if !self.policy.enabled {
            return None;
        }
        let payload = self
            .storage
            .document_cache_get(
                canonical_url,
                content_hash,
                extractor_version,
                now(),
                self.policy.ttl_seconds,
            )
            .await
            .ok()??;
        serde_json::from_str(&payload).ok()
    }

    pub async fn latest(&self, canonical_url: &str, extractor_version: &str) -> Option<Document> {
        if !self.policy.enabled {
            return None;
        }
        let payload = self
            .storage
            .document_cache_get_latest(
                canonical_url,
                extractor_version,
                now(),
                self.policy.ttl_seconds,
            )
            .await
            .ok()??;
        serde_json::from_str(&payload).ok()
    }

    pub async fn put(
        &self,
        document: &Document,
        extractor_version: &str,
        storage_rights: bool,
    ) -> bool {
        if !self.policy.enabled || !storage_rights {
            return false;
        }
        let mut stored = document.clone();
        if !self.policy.store_content {
            stored.content = None;
        }
        let Ok(payload) = serde_json::to_string(&stored) else {
            return false;
        };
        self.storage
            .document_cache_put(
                document.canonical_url.0.as_str(),
                &document.content_hash,
                extractor_version,
                &payload,
                now(),
                self.policy.ttl_seconds,
                self.policy.max_entries,
                self.policy.max_bytes,
            )
            .await
            .is_ok()
    }

    pub async fn stats(&self) -> CacheStats {
        self.storage
            .document_cache_stats()
            .await
            .unwrap_or_default()
    }
    pub async fn purge(&self) -> u64 {
        self.storage.document_cache_purge().await.unwrap_or(0)
    }
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |value| value.as_secs() as i64)
}
