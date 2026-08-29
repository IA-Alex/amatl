use crate::model::Document;
use crate::storage::{CacheStats, SqliteStorage};

#[derive(Clone, Debug)]
pub struct DocumentCachePolicy {
    pub enabled: bool,
    pub ttl_seconds: u64,
    pub max_entries: u64,
    pub max_bytes: u64,
    pub store_content: bool,
    /// When set, stale entries within this window are returned while
    /// a background revalidation is triggered.
    pub stale_while_revalidate_seconds: u64,
    /// Identity of the inference vector space in effect, when Deep ranking
    /// uses one. Entries are namespaced by it so changing the embedding
    /// backend or its width cannot silently reuse artifacts produced under a
    /// different vector space; the old rows simply stop matching and expire.
    pub model_version: Option<String>,
}

impl Default for DocumentCachePolicy {
    fn default() -> Self {
        Self {
            enabled: false,
            ttl_seconds: 86_400,
            max_entries: 10_000,
            max_bytes: 268_435_456,
            store_content: false,
            stale_while_revalidate_seconds: 0,
            model_version: None,
        }
    }
}

#[derive(Clone)]
pub struct DocumentCache {
    storage: SqliteStorage,
    policy: DocumentCachePolicy,
    counters: Option<std::sync::Arc<crate::cache::CacheCounters>>,
}

impl DocumentCache {
    pub fn new(storage: SqliteStorage, policy: DocumentCachePolicy) -> Self {
        Self {
            storage,
            policy,
            counters: None,
        }
    }

    /// Report hits and misses of this cache into shared counters.
    pub fn with_counters(mut self, counters: std::sync::Arc<crate::cache::CacheCounters>) -> Self {
        self.counters = Some(counters);
        self
    }

    /// Cache namespace for one extractor version under the active vector
    /// space. Keys are opaque to SQLite, so composing them here keeps the
    /// storage schema unchanged.
    fn namespace(&self, extractor_version: &str) -> String {
        match &self.policy.model_version {
            Some(model) => format!("{extractor_version}#{model}"),
            None => extractor_version.to_owned(),
        }
    }

    fn record(&self, hit: bool) {
        if let Some(counters) = &self.counters {
            counters.record_document(hit);
        }
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
        let document = self
            .storage
            .document_cache_get(
                canonical_url,
                content_hash,
                &self.namespace(extractor_version),
                now(),
                self.policy.ttl_seconds,
            )
            .await
            .ok()
            .flatten()
            .and_then(|payload| serde_json::from_str(&payload).ok());
        self.record(document.is_some());
        document
    }

    pub async fn latest(&self, canonical_url: &str, extractor_version: &str) -> Option<Document> {
        if !self.policy.enabled {
            return None;
        }
        let payload = self
            .storage
            .document_cache_get_latest(
                canonical_url,
                &self.namespace(extractor_version),
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
        etag: Option<&str>,
        last_modified: Option<&str>,
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
                &self.namespace(extractor_version),
                &payload,
                now(),
                self.policy.ttl_seconds,
                self.policy.max_entries,
                self.policy.max_bytes,
                etag,
                last_modified,
            )
            .await
            .is_ok()
    }

    /// Get a cached document with revalidation metadata.
    ///
    /// Returns the document along with ETag and Last-Modified headers so the
    /// caller can perform conditional revalidation against the origin.
    /// If `stale_while_revalidate_seconds` is configured, stale-but-valid
    /// entries are returned while a background refresh is triggered.
    pub async fn get_with_revalidation(
        &self,
        canonical_url: &str,
        content_hash: &str,
        extractor_version: &str,
    ) -> Option<(Document, Option<String>, Option<String>, bool)> {
        if !self.policy.enabled {
            return None;
        }
        let cached = self
            .storage
            .document_cache_get_with_revalidation(
                canonical_url,
                content_hash,
                &self.namespace(extractor_version),
                now(),
                self.policy.ttl_seconds,
                self.policy.stale_while_revalidate_seconds,
            )
            .await
            .ok()
            .flatten();
        self.record(cached.is_some());
        let cached = cached?;
        let document: Document = serde_json::from_str(&cached.payload).ok()?;
        Some((document, cached.etag, cached.last_modified, cached.fresh))
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
