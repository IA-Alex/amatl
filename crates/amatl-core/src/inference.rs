//! Local inference contract for AMATL.
//!
//! Ranking v2 accepts two optional backends ([`crate::SemanticScorer`] and
//! [`crate::DeepReranker`]). This module defines the embedding contract behind
//! them and ships one bounded, deterministic, fully local implementation so
//! `data_policy.inference = "local_only"` has a real backend instead of an
//! empty extension point.
//!
//! Guarantees kept by every backend built here:
//!
//! * **No egress.** Nothing in this module opens a socket, spawns a process or
//!   reads model files; `local_hashing_v1` is pure computation over the text
//!   already retrieved by Deep.
//! * **Determinism.** The same query and documents always produce the same
//!   scores, so the Ranking v2 quality gate stays reproducible.
//! * **Bounded cost.** Input length and document count are capped by
//!   configuration; exceeding them fails the optional backend instead of
//!   inflating the Deep budget, and Deep degrades to lexical ranking.
//!
//! `remote_explicit` is the only mode that may leave the machine. It resolves
//! to [`RemoteEmbeddingBackend`], which is governed rather than implicit:
//!
//! * it is built only when [`DataPolicyConfig::allows_remote_inference`] is
//!   true, so `isolated`, `deny` and the other two inference modes can never
//!   reach it;
//! * the endpoint comes from configuration, must be absolute HTTPS (loopback
//!   may use HTTP for a self-hosted server), and must carry no credentials in
//!   the URL;
//! * batch size, input length, response width and timeout are bounded by
//!   configuration, and any deviation fails the optional backend so Deep
//!   degrades to lexical ranking instead of retrying;
//! * a missing endpoint or credential fails closed with
//!   [`InferenceError::RemoteBackendUnavailable`].
//!
//! What crosses the boundary in that mode is exactly the query plus the
//! bounded document text Deep already retrieved; nothing else is sent.

use crate::config::{DataPolicyConfig, InferenceConfig, InferenceMode};
use crate::model::{Document, Query};
use crate::providers::{HttpRequest, HttpTransport};
use crate::ranking_v2::{DeepReranker, RankingV2Error, SemanticScorer};
use crate::text::normalized_text;
use async_trait::async_trait;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::sync::Arc;
use thiserror::Error;

/// Identifier of the built-in offline embedding backend.
pub const LOCAL_EMBEDDING_BACKEND_ID: &str = "local_hashing_v1";
/// Identifier of the built-in offline reranker.
pub const LOCAL_RERANKER_ID: &str = "lexical_coverage_v1";
/// Narrowest accepted embedding width for the local backend.
pub const MINIMUM_EMBEDDING_DIMENSIONS: usize = 32;
/// Widest accepted embedding width for the local backend.
pub const MAXIMUM_EMBEDDING_DIMENSIONS: usize = 4_096;

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum InferenceError {
    #[error("inference is disabled by data policy")]
    Disabled,
    #[error("remote inference has no available backend")]
    RemoteBackendUnavailable,
    #[error("unknown inference backend: {0}")]
    UnknownBackend(String),
    #[error("invalid inference backend limit")]
    InvalidLimit,
    #[error("inference input exceeds the configured bound")]
    InputTooLarge,
    #[error("remote inference endpoint is invalid: {0}")]
    InvalidEndpoint(String),
    #[error("remote inference request failed")]
    RemoteRequestFailed,
    #[error("remote inference response did not match the embedding contract")]
    RemoteResponseInvalid,
    #[error("local model file is missing, malformed or incompatible")]
    ModelUnavailable,
    #[error("embedding backend returned a result that violates the embedding contract")]
    BackendContractViolation,
}

/// Contract every embedding backend must honor.
///
/// Implementors are free to be remote or model-backed, but a backend handed to
/// AMATL under `local_only` must not perform network or filesystem access.
#[async_trait]
pub trait EmbeddingBackend: Send + Sync {
    /// Stable backend identifier recorded in ranking explanations and logs.
    fn id(&self) -> &str;
    /// Width of every produced vector.
    fn dimensions(&self) -> usize;
    /// Whether producing a vector leaves the machine.
    fn is_remote(&self) -> bool {
        false
    }
    /// Embed inputs in order, returning one L2-normalized vector per input.
    async fn embed(&self, inputs: &[String]) -> Result<Vec<Vec<f32>>, InferenceError>;
}

/// Offline embedding backend based on signed feature hashing.
///
/// Unigrams and adjacent bigrams are hashed into a fixed-width vector with
/// sublinear term frequency weighting, then L2-normalized. It carries no model
/// weights, so it is reproducible across machines and releases.
pub struct LocalHashingEmbedder {
    dimensions: usize,
    max_input_chars: usize,
}

impl LocalHashingEmbedder {
    pub fn new(dimensions: usize, max_input_chars: usize) -> Result<Self, InferenceError> {
        if !(MINIMUM_EMBEDDING_DIMENSIONS..=MAXIMUM_EMBEDDING_DIMENSIONS).contains(&dimensions)
            || max_input_chars == 0
        {
            return Err(InferenceError::InvalidLimit);
        }
        Ok(Self {
            dimensions,
            max_input_chars,
        })
    }
}

#[async_trait]
impl EmbeddingBackend for LocalHashingEmbedder {
    fn id(&self) -> &str {
        LOCAL_EMBEDDING_BACKEND_ID
    }

    fn dimensions(&self) -> usize {
        self.dimensions
    }

    async fn embed(&self, inputs: &[String]) -> Result<Vec<Vec<f32>>, InferenceError> {
        inputs
            .iter()
            .map(|input| {
                let bounded = bounded_text(input, self.max_input_chars);
                let tokens = terms(&bounded);
                let mut weights: BTreeMap<String, f64> = BTreeMap::new();
                for token in &tokens {
                    *weights.entry(token.clone()).or_insert(0.0) += 1.0;
                }
                for pair in tokens.windows(2) {
                    *weights
                        .entry(format!("{} {}", pair[0], pair[1]))
                        .or_insert(0.0) += 0.5;
                }
                let mut vector = vec![0.0_f32; self.dimensions];
                for (feature, frequency) in weights {
                    let digest = feature_hash(&feature);
                    let index = (digest % self.dimensions as u64) as usize;
                    let sign = if digest & (1 << 63) == 0 { 1.0 } else { -1.0 };
                    vector[index] += (sign * (1.0 + frequency.ln())) as f32;
                }
                Ok(l2_normalized(vector))
            })
            .collect()
    }
}

/// Identifier of the local word-vector model backend.
pub const LOCAL_MODEL_BACKEND_ID: &str = "local_model_v1";
/// Largest word-vector model file the local backend will load.
///
/// Parsing expands the file several times over in memory, so this is a hard
/// ceiling rather than a hint: exceeding it fails the optional backend and Deep
/// degrades, which is preferable to being killed by the OOM reaper at startup.
pub const MAX_LOCAL_MODEL_BYTES: u64 = 512 * 1024 * 1024;

/// Offline embedding backend backed by a word-vector model file.
///
/// The model is a plain-text table of `token v0 v1 … vN` lines. Each input is
/// tokenized and the vectors of every token present in the model are summed
/// (with sublinear term-frequency weighting), then L2-normalized. Unlike the
/// hashing backend this carries real weights, so it is a genuine model-backed
/// embedder rather than a feature hash.
pub struct LocalModelEmbedder {
    dimensions: usize,
    max_input_chars: usize,
    vectors: BTreeMap<String, Vec<f32>>,
}

impl LocalModelEmbedder {
    /// Load a word-vector model from `path`, validating width and limits.
    pub fn load(
        path: &std::path::Path,
        dimensions: usize,
        max_input_chars: usize,
    ) -> Result<Self, InferenceError> {
        if !(MINIMUM_EMBEDDING_DIMENSIONS..=MAXIMUM_EMBEDDING_DIMENSIONS).contains(&dimensions)
            || max_input_chars == 0
        {
            return Err(InferenceError::InvalidLimit);
        }
        // Bound the model before reading it. A word-vector table expands to
        // several times its on-disk size once parsed into a map of `Vec<f32>`,
        // so an oversized or mistyped path would otherwise be an OOM at
        // startup. `max_input_chars` already bounds the input side; the model
        // side was unbounded.
        let size = std::fs::metadata(path)
            .map_err(|_| InferenceError::ModelUnavailable)?
            .len();
        if size > MAX_LOCAL_MODEL_BYTES {
            tracing::warn!(
                target: "amatl::inference",
                path = %path.display(),
                size_bytes = size,
                limit_bytes = MAX_LOCAL_MODEL_BYTES,
                "local model file exceeds the size limit"
            );
            return Err(InferenceError::ModelUnavailable);
        }

        let file = std::fs::File::open(path).map_err(|_| InferenceError::ModelUnavailable)?;
        let reader = std::io::BufReader::new(file);
        let mut vectors = BTreeMap::new();
        for line in std::io::BufRead::lines(reader) {
            let line = line.map_err(|_| InferenceError::ModelUnavailable)?;
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut fields = line.split_whitespace();
            let Some(token) = fields.next() else {
                continue;
            };
            let mut vector = Vec::with_capacity(dimensions);
            for field in fields {
                let value: f32 = field
                    .parse()
                    .map_err(|_| InferenceError::ModelUnavailable)?;
                vector.push(value);
            }
            if vector.len() != dimensions {
                return Err(InferenceError::ModelUnavailable);
            }
            vectors.insert(token.to_owned(), vector);
        }
        if vectors.is_empty() {
            return Err(InferenceError::ModelUnavailable);
        }
        Ok(Self {
            dimensions,
            max_input_chars,
            vectors,
        })
    }
}

#[async_trait]
impl EmbeddingBackend for LocalModelEmbedder {
    fn id(&self) -> &str {
        LOCAL_MODEL_BACKEND_ID
    }
    fn dimensions(&self) -> usize {
        self.dimensions
    }
    async fn embed(&self, inputs: &[String]) -> Result<Vec<Vec<f32>>, InferenceError> {
        inputs
            .iter()
            .map(|input| {
                let bounded = bounded_text(input, self.max_input_chars);
                let tokens = terms(&bounded);
                let mut counts: BTreeMap<String, usize> = BTreeMap::new();
                for token in &tokens {
                    *counts.entry(token.clone()).or_insert(0) += 1;
                }
                let mut vector = vec![0.0_f32; self.dimensions];
                let mut weight_sum = 0.0_f64;
                for (token, count) in counts {
                    let Some(model_vector) = self.vectors.get(&token) else {
                        continue;
                    };
                    let weight = 1.0 + (count as f64).ln();
                    for (slot, value) in vector.iter_mut().zip(model_vector.iter()) {
                        *slot += (*value as f64 * weight) as f32;
                    }
                    weight_sum += weight;
                }
                if weight_sum == 0.0 {
                    return Ok(vector);
                }
                Ok(l2_normalized(vector))
            })
            .collect()
    }
}

/// Persistent embedding cache keyed by text hash and namespaced by the vector
/// space identity, so artifacts are never reused across backends or widths.
///
/// The cache is bounded and evicts least-recently-used entries. Insertion order
/// is persisted alongside the vectors, so eviction stays meaningful across
/// runs; a plain map would restore in hash order and evict essentially at
/// random. It is written back to disk on drop, giving reuse between executions
/// without unbounded growth.
pub struct EmbeddingCache {
    path: std::path::PathBuf,
    namespace: String,
    capacity: usize,
    entries: BTreeMap<String, Vec<f32>>,
    order: std::collections::VecDeque<String>,
}

/// On-disk cache layout. Versioned so a future change can be detected rather
/// than silently discarded, and ordered so LRU survives a restart.
#[derive(serde::Serialize, serde::Deserialize)]
struct EmbeddingCacheFile {
    version: u32,
    /// Least-recently-used first.
    entries: Vec<(String, Vec<f32>)>,
}

/// Current [`EmbeddingCacheFile`] layout version.
const EMBEDDING_CACHE_VERSION: u32 = 1;

impl EmbeddingCache {
    pub fn load(path: std::path::PathBuf, namespace: String, capacity: usize) -> Self {
        let mut entries = BTreeMap::new();
        let mut order = std::collections::VecDeque::new();
        match std::fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice::<EmbeddingCacheFile>(&bytes) {
                Ok(file) if file.version == EMBEDDING_CACHE_VERSION => {
                    for (key, vector) in file.entries {
                        order.push_back(key.clone());
                        entries.insert(key, vector);
                    }
                }
                Ok(file) => {
                    tracing::warn!(
                        target: "amatl::inference",
                        path = %path.display(),
                        found = file.version,
                        expected = EMBEDDING_CACHE_VERSION,
                        "embedding cache has an unknown layout version; starting empty"
                    );
                }
                Err(error) => {
                    // Previously swallowed: a truncated file silently threw the
                    // whole cache away with no signal to the operator.
                    tracing::warn!(
                        target: "amatl::inference",
                        path = %path.display(),
                        error = %error,
                        "embedding cache is unreadable; starting empty"
                    );
                }
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                tracing::warn!(
                    target: "amatl::inference",
                    path = %path.display(),
                    error = %error,
                    "embedding cache could not be read; starting empty"
                );
            }
        }
        while order.len() > capacity {
            if let Some(oldest) = order.pop_front() {
                entries.remove(&oldest);
            }
        }
        Self {
            path,
            namespace,
            capacity,
            entries,
            order,
        }
    }

    /// Look up `text`, marking the entry as most recently used.
    pub fn get(&mut self, text: &str) -> Option<Vec<f32>> {
        let key = cache_key(&self.namespace, text);
        let vector = self.entries.get(&key).cloned()?;
        if let Some(position) = self.order.iter().position(|entry| entry == &key) {
            self.order.remove(position);
        }
        self.order.push_back(key);
        Some(vector)
    }

    pub fn insert(&mut self, text: &str, vector: Vec<f32>) {
        let key = cache_key(&self.namespace, text);
        if self.entries.insert(key.clone(), vector).is_some() {
            // Refresh recency rather than returning early, so a repeatedly used
            // entry is not evicted ahead of one touched once.
            if let Some(position) = self.order.iter().position(|entry| entry == &key) {
                self.order.remove(position);
            }
        }
        self.order.push_back(key);
        while self.order.len() > self.capacity {
            if let Some(oldest) = self.order.pop_front() {
                self.entries.remove(&oldest);
            }
        }
    }

    pub fn save(&self) {
        if let Some(parent) = self.path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let file = EmbeddingCacheFile {
            version: EMBEDDING_CACHE_VERSION,
            entries: self
                .order
                .iter()
                .filter_map(|key| {
                    self.entries
                        .get(key)
                        .map(|vector| (key.clone(), vector.clone()))
                })
                .collect(),
        };
        let Ok(json) = serde_json::to_vec(&file) else {
            return;
        };
        // Write-then-rename: `fs::write` truncates first, so a crash mid-write
        // leaves a truncated file that the next run has to discard entirely.
        let temporary = self.path.with_extension("tmp");
        if std::fs::write(&temporary, json).is_ok()
            && std::fs::rename(&temporary, &self.path).is_err()
        {
            let _ = std::fs::remove_file(&temporary);
        }
    }
}

impl Drop for EmbeddingCache {
    fn drop(&mut self) {
        self.save();
    }
}

fn cache_key(namespace: &str, text: &str) -> String {
    let digest = Sha256::digest(text.as_bytes());
    format!("{namespace}:{:x}", digest)
}

/// Wraps an [`EmbeddingBackend`] with a persistent [`EmbeddingCache`], serving
/// previously computed vectors without re-running the backend.
pub struct CachedEmbeddingBackend {
    inner: Arc<dyn EmbeddingBackend>,
    cache: Arc<std::sync::Mutex<EmbeddingCache>>,
}

impl CachedEmbeddingBackend {
    pub fn new(inner: Arc<dyn EmbeddingBackend>, cache: EmbeddingCache) -> Self {
        Self {
            inner,
            cache: Arc::new(std::sync::Mutex::new(cache)),
        }
    }
}

#[async_trait]
impl EmbeddingBackend for CachedEmbeddingBackend {
    fn id(&self) -> &str {
        self.inner.id()
    }
    fn dimensions(&self) -> usize {
        self.inner.dimensions()
    }
    fn is_remote(&self) -> bool {
        self.inner.is_remote()
    }
    async fn embed(&self, inputs: &[String]) -> Result<Vec<Vec<f32>>, InferenceError> {
        let mut missing_indices = Vec::new();
        let mut missing_inputs = Vec::new();
        let mut result: Vec<Option<Vec<f32>>> = vec![None; inputs.len()];
        {
            // A panic elsewhere must not poison the cache into permanently
            // failing every later embed call; the repo already treats its other
            // shared caches this way.
            let mut cache = self.cache.lock().unwrap_or_else(|error| error.into_inner());
            for (index, input) in inputs.iter().enumerate() {
                if let Some(vector) = cache.get(input) {
                    result[index] = Some(vector);
                } else {
                    missing_indices.push(index);
                    missing_inputs.push(input.clone());
                }
            }
        }
        if !missing_inputs.is_empty() {
            let computed = self.inner.embed(&missing_inputs).await?;
            if computed.len() != missing_inputs.len() {
                return Err(InferenceError::BackendContractViolation);
            }
            let mut cache = self.cache.lock().unwrap_or_else(|error| error.into_inner());
            for (index, vector) in missing_indices.into_iter().zip(computed) {
                cache.insert(&inputs[index], vector.clone());
                result[index] = Some(vector);
            }
        }
        result
            .into_iter()
            .map(|vector| vector.ok_or(InferenceError::BackendContractViolation))
            .collect()
    }
}

/// Identifier prefix of the governed remote embedding backend.
pub const REMOTE_EMBEDDING_BACKEND_ID: &str = "remote_embeddings_v1";
/// Largest batch a remote backend may be configured to send in one request.
pub const MAXIMUM_REMOTE_BATCH: usize = 256;

/// Governed remote embedding backend.
///
/// Speaks the widely implemented `{"model": …, "input": [ … ]}` →
/// `{"data": [{"embedding": [ … ]}]}` JSON shape, so a self-hosted or vendor
/// endpoint can be pointed at without a bespoke adapter. Everything about the
/// call is bounded by configuration and validated on the way out and back in;
/// see the module documentation for the egress contract.
pub struct RemoteEmbeddingBackend {
    endpoint: url::Url,
    model: String,
    credential: Option<String>,
    dimensions: usize,
    max_batch: usize,
    max_input_chars: usize,
    timeout_ms: u64,
    transport: Arc<dyn HttpTransport>,
}

impl RemoteEmbeddingBackend {
    /// Build the backend from configuration, failing closed on any deviation.
    pub fn new(
        config: &InferenceConfig,
        transport: Arc<dyn HttpTransport>,
    ) -> Result<Self, InferenceError> {
        if !(MINIMUM_EMBEDDING_DIMENSIONS..=MAXIMUM_EMBEDDING_DIMENSIONS)
            .contains(&config.embedding_dimensions)
            || config.max_input_chars == 0
            || config.max_documents == 0
            || !(1..=MAXIMUM_REMOTE_BATCH).contains(&config.remote_max_batch)
            || !(100..=60_000).contains(&config.remote_timeout_ms)
        {
            return Err(InferenceError::InvalidLimit);
        }
        let credential = config
            .remote_credential_env
            .as_deref()
            .map(|name| {
                std::env::var(name)
                    .ok()
                    .filter(|value| !value.is_empty())
                    .ok_or(InferenceError::RemoteBackendUnavailable)
            })
            .transpose()?;
        Self::with_credential(config, transport, credential)
    }

    /// Build the backend with an already resolved credential.
    ///
    /// [`Self::new`] reads it from `remote_credential_env`; this entry point
    /// exists for embedders that hold the secret themselves, and keeps the
    /// validation identical either way.
    pub fn with_credential(
        config: &InferenceConfig,
        transport: Arc<dyn HttpTransport>,
        credential: Option<String>,
    ) -> Result<Self, InferenceError> {
        let endpoint = validate_remote_endpoint(
            config
                .remote_endpoint
                .as_deref()
                .ok_or(InferenceError::RemoteBackendUnavailable)?,
        )?;
        if credential.as_deref().is_some_and(str::is_empty) {
            return Err(InferenceError::RemoteBackendUnavailable);
        }
        Ok(Self {
            endpoint,
            model: config
                .remote_model
                .clone()
                .ok_or(InferenceError::RemoteBackendUnavailable)?,
            credential,
            dimensions: config.embedding_dimensions,
            max_batch: config.remote_max_batch,
            max_input_chars: config.max_input_chars,
            timeout_ms: config.remote_timeout_ms,
            transport,
        })
    }

    fn request_body(&self, batch: &[String]) -> Result<Vec<u8>, InferenceError> {
        let inputs = batch
            .iter()
            .map(|input| bounded_text(input, self.max_input_chars))
            .collect::<Vec<_>>();
        serde_json::to_vec(&serde_json::json!({
            "model": self.model,
            "input": inputs,
        }))
        .map_err(|_| InferenceError::RemoteRequestFailed)
    }

    async fn embed_batch(&self, batch: &[String]) -> Result<Vec<Vec<f32>>, InferenceError> {
        let mut headers = vec![("accept".to_string(), "application/json".to_string())];
        if let Some(credential) = &self.credential {
            headers.push(("authorization".into(), format!("Bearer {credential}")));
        }
        let response = self
            .transport
            .execute(HttpRequest::post_json(
                self.endpoint.clone(),
                headers,
                self.timeout_ms,
                self.request_body(batch)?,
            ))
            .await
            .map_err(|_| InferenceError::RemoteRequestFailed)?;
        if response.status != 200 {
            return Err(InferenceError::RemoteRequestFailed);
        }
        parse_embedding_response(&response.body, batch.len(), self.dimensions)
    }
}

#[async_trait]
impl EmbeddingBackend for RemoteEmbeddingBackend {
    fn id(&self) -> &str {
        REMOTE_EMBEDDING_BACKEND_ID
    }

    fn dimensions(&self) -> usize {
        self.dimensions
    }

    fn is_remote(&self) -> bool {
        true
    }

    async fn embed(&self, inputs: &[String]) -> Result<Vec<Vec<f32>>, InferenceError> {
        if inputs.is_empty() {
            return Ok(vec![]);
        }
        let mut vectors = Vec::with_capacity(inputs.len());
        for batch in inputs.chunks(self.max_batch) {
            vectors.extend(self.embed_batch(batch).await?);
        }
        if vectors.len() != inputs.len() {
            return Err(InferenceError::RemoteResponseInvalid);
        }
        Ok(vectors)
    }
}

/// Accept only an absolute endpoint that carries no credentials and either
/// uses HTTPS or targets loopback, where a self-hosted server is legitimate.
pub fn validate_remote_endpoint(value: &str) -> Result<url::Url, InferenceError> {
    let url = url::Url::parse(value)
        .map_err(|_| InferenceError::InvalidEndpoint("endpoint is not an absolute URL".into()))?;
    if !url.username().is_empty() || url.password().is_some() {
        return Err(InferenceError::InvalidEndpoint(
            "endpoint must not embed credentials".into(),
        ));
    }
    let loopback = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    match url.scheme() {
        "https" => Ok(url),
        "http" if loopback => Ok(url),
        _ => Err(InferenceError::InvalidEndpoint(
            "endpoint must be https, or http on loopback".into(),
        )),
    }
}

fn parse_embedding_response(
    body: &[u8],
    expected: usize,
    dimensions: usize,
) -> Result<Vec<Vec<f32>>, InferenceError> {
    let payload: serde_json::Value =
        serde_json::from_slice(body).map_err(|_| InferenceError::RemoteResponseInvalid)?;
    let data = payload
        .get("data")
        .and_then(serde_json::Value::as_array)
        .ok_or(InferenceError::RemoteResponseInvalid)?;
    if data.len() != expected {
        return Err(InferenceError::RemoteResponseInvalid);
    }
    data.iter()
        .map(|entry| {
            let vector = entry
                .get("embedding")
                .and_then(serde_json::Value::as_array)
                .ok_or(InferenceError::RemoteResponseInvalid)?;
            if vector.len() != dimensions {
                return Err(InferenceError::RemoteResponseInvalid);
            }
            let vector = vector
                .iter()
                .map(|value| {
                    value
                        .as_f64()
                        .filter(|value| value.is_finite())
                        .map(|value| value as f32)
                        .ok_or(InferenceError::RemoteResponseInvalid)
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok(l2_normalized(vector))
        })
        .collect()
}

/// Semantic scorer backed by an [`EmbeddingBackend`].
///
/// Scores are cosine similarities between the normalized query vector and each
/// document vector, clamped to `[0, 1]`; negative similarity means "unrelated"
/// and collapses to zero.
pub struct EmbeddingSemanticScorer {
    backend: Arc<dyn EmbeddingBackend>,
    max_documents: usize,
}

impl EmbeddingSemanticScorer {
    pub fn new(backend: Arc<dyn EmbeddingBackend>, max_documents: usize) -> Self {
        Self {
            backend,
            max_documents,
        }
    }
}

#[async_trait]
impl SemanticScorer for EmbeddingSemanticScorer {
    fn name(&self) -> &str {
        self.backend.id()
    }

    async fn score(
        &self,
        query: &Query,
        documents: &[Document],
    ) -> Result<Vec<f64>, RankingV2Error> {
        if documents.len() > self.max_documents {
            return Err(RankingV2Error::Backend);
        }
        let mut inputs = Vec::with_capacity(documents.len() + 1);
        inputs.push(query.normalized_query.clone());
        inputs.extend(documents.iter().map(document_text));
        let vectors = self
            .backend
            .embed(&inputs)
            .await
            .map_err(|_| RankingV2Error::Backend)?;
        let Some((query_vector, document_vectors)) = vectors.split_first() else {
            return Err(RankingV2Error::Backend);
        };
        if document_vectors.len() != documents.len() {
            return Err(RankingV2Error::Backend);
        }
        Ok(document_vectors
            .iter()
            .map(|vector| dot(query_vector, vector).clamp(0.0, 1.0))
            .collect())
    }
}

/// Offline reranker combining query-term coverage with the prior relevance.
///
/// It is a deterministic lexical heuristic, not a cross-encoder: coverage
/// rewards documents that mention every distinct query term, and the prior keeps
/// the upstream relevance signal from being discarded.
pub struct LexicalCoverageReranker {
    max_documents: usize,
    prior_weight: f64,
}

impl LexicalCoverageReranker {
    pub fn new(max_documents: usize, prior_weight: f64) -> Result<Self, InferenceError> {
        if max_documents == 0 || !(0.0..=1.0).contains(&prior_weight) {
            return Err(InferenceError::InvalidLimit);
        }
        Ok(Self {
            max_documents,
            prior_weight,
        })
    }
}

#[async_trait]
impl DeepReranker for LexicalCoverageReranker {
    fn name(&self) -> &str {
        LOCAL_RERANKER_ID
    }

    async fn score(
        &self,
        query: &Query,
        documents: &[Document],
        relevance: &[f64],
    ) -> Result<Vec<f64>, RankingV2Error> {
        if documents.len() > self.max_documents || relevance.len() != documents.len() {
            return Err(RankingV2Error::Backend);
        }
        let query_terms = crate::text::tokens(&query.normalized_query);
        Ok(documents
            .iter()
            .enumerate()
            .map(|(index, document)| {
                let coverage = if query_terms.is_empty() {
                    0.0
                } else {
                    let document_terms = crate::text::tokens(&document_text(document));
                    query_terms
                        .iter()
                        .filter(|term| document_terms.contains(*term))
                        .count() as f64
                        / query_terms.len() as f64
                };
                ((1.0 - self.prior_weight) * coverage + self.prior_weight * relevance[index])
                    .clamp(0.0, 1.0)
            })
            .collect())
    }
}

/// Resolved inference capability for a configuration.
#[derive(Clone)]
pub struct InferenceRuntime {
    backend: Arc<dyn EmbeddingBackend>,
    max_documents: usize,
    reranker_prior_weight: f64,
}

impl InferenceRuntime {
    /// Resolve the runtime for a data policy, or `None` when inference is off.
    ///
    /// `remote_explicit` fails closed: AMATL ships no remote backend, so the
    /// caller must degrade instead of leaking corpus text.
    pub fn from_policy(
        policy: &DataPolicyConfig,
        config: &InferenceConfig,
        transport: Option<Arc<dyn HttpTransport>>,
    ) -> Result<Option<Self>, InferenceError> {
        match policy.inference {
            InferenceMode::Disabled => Ok(None),
            InferenceMode::LocalOnly => Self::local(config).map(Some),
            InferenceMode::RemoteExplicit => {
                // Belt and braces: the mode alone never authorizes egress.
                if !policy.allows_remote_inference() {
                    return Err(InferenceError::RemoteBackendUnavailable);
                }
                let transport = transport.ok_or(InferenceError::RemoteBackendUnavailable)?;
                Self::remote(config, transport).map(Some)
            }
        }
    }

    /// Build the governed remote runtime described by `config`.
    pub fn remote(
        config: &InferenceConfig,
        transport: Arc<dyn HttpTransport>,
    ) -> Result<Self, InferenceError> {
        let backend = RemoteEmbeddingBackend::new(config, transport)?;
        Ok(Self {
            backend: Arc::new(backend),
            max_documents: config.max_documents,
            reranker_prior_weight: config.reranker_prior_weight,
        })
    }

    /// Build the offline runtime described by `config`.
    pub fn local(config: &InferenceConfig) -> Result<Self, InferenceError> {
        if config.max_documents == 0 {
            return Err(InferenceError::InvalidLimit);
        }
        let backend: Arc<dyn EmbeddingBackend> = match config.backend.as_str() {
            LOCAL_EMBEDDING_BACKEND_ID => Arc::new(LocalHashingEmbedder::new(
                config.embedding_dimensions,
                config.max_input_chars,
            )?),
            LOCAL_MODEL_BACKEND_ID => {
                let path = config
                    .local_model_path
                    .as_deref()
                    .ok_or(InferenceError::ModelUnavailable)?;
                Arc::new(LocalModelEmbedder::load(
                    std::path::Path::new(path),
                    config.embedding_dimensions,
                    config.max_input_chars,
                )?)
            }
            other => return Err(InferenceError::UnknownBackend(other.into())),
        };
        let backend = match &config.local_cache_path {
            Some(path) => {
                let namespace = format!("{}@{}", backend.id(), backend.dimensions());
                Arc::new(CachedEmbeddingBackend::new(
                    backend,
                    EmbeddingCache::load(
                        std::path::PathBuf::from(path),
                        namespace,
                        config.local_model_batch.max(1) * 64,
                    ),
                )) as Arc<dyn EmbeddingBackend>
            }
            None => backend,
        };
        Ok(Self {
            backend,
            max_documents: config.max_documents,
            reranker_prior_weight: config.reranker_prior_weight,
        })
    }

    pub fn backend_id(&self) -> &str {
        self.backend.id()
    }

    /// Whether scoring sends corpus text to a third party.
    pub fn is_remote(&self) -> bool {
        self.backend.is_remote()
    }

    /// Identity of the vector space this runtime produces.
    ///
    /// Anything derived from embeddings must be namespaced by this key: two
    /// runtimes with different backends or widths are not interchangeable, and
    /// reusing cached artifacts across them would compare vectors that never
    /// shared a space. It is stable for a given configuration.
    pub fn version_key(&self) -> String {
        format!("{}@{}", self.backend.id(), self.backend.dimensions())
    }

    pub fn embedding_backend(&self) -> Arc<dyn EmbeddingBackend> {
        self.backend.clone()
    }

    pub fn semantic_scorer(&self) -> Arc<dyn SemanticScorer> {
        Arc::new(EmbeddingSemanticScorer::new(
            self.backend.clone(),
            self.max_documents,
        ))
    }

    /// Reranker for Deep, chosen by whether the backend carries real semantics.
    ///
    /// `local_hashing_v1` is a signed feature hash, not a model: the cosine
    /// between two hashes is a far weaker signal than lexical coverage. On the
    /// labeled ranking corpus it scores measurably worse (nDCG@3 0.925 against
    /// 1.000; see `ranking_v2::reranker_measurement`), so it must not be the
    /// default. A remote backend is excluded for a different reason: reranking
    /// through it would ship the text of every candidate document to a third
    /// party on every Deep call, which is a change of privacy posture and needs
    /// its own opt-in rather than riding on the inference mode.
    pub fn reranker(&self) -> Result<Arc<dyn DeepReranker>, InferenceError> {
        let semantic_backend =
            !self.backend.is_remote() && self.backend.id() != LOCAL_EMBEDDING_BACKEND_ID;
        if !semantic_backend {
            return LexicalCoverageReranker::new(self.max_documents, self.reranker_prior_weight)
                .map(|value| Arc::new(value) as Arc<dyn DeepReranker>);
        }
        EmbeddingReranker::new(
            self.backend.clone(),
            self.max_documents,
            self.reranker_prior_weight,
        )
        .map(|value| Arc::new(value) as Arc<dyn DeepReranker>)
    }
}

/// Semantic reranker backed by an [`EmbeddingBackend`].
///
/// Scores combine the query–document cosine similarity (a genuine semantic
/// signal) with the upstream relevance prior. When the embedding backend fails
/// — for example because the local model file is missing — it degrades to the
/// deterministic [`LexicalCoverageReranker`] instead of failing Deep.
pub struct EmbeddingReranker {
    backend: Arc<dyn EmbeddingBackend>,
    max_documents: usize,
    prior_weight: f64,
    lexical: LexicalCoverageReranker,
}

impl EmbeddingReranker {
    pub fn new(
        backend: Arc<dyn EmbeddingBackend>,
        max_documents: usize,
        prior_weight: f64,
    ) -> Result<Self, InferenceError> {
        if max_documents == 0 || !(0.0..=1.0).contains(&prior_weight) {
            return Err(InferenceError::InvalidLimit);
        }
        Ok(Self {
            backend,
            max_documents,
            prior_weight,
            lexical: LexicalCoverageReranker::new(max_documents, prior_weight)?,
        })
    }
}

#[async_trait]
impl DeepReranker for EmbeddingReranker {
    fn name(&self) -> &str {
        "embedding_semantic_v1"
    }

    async fn score(
        &self,
        query: &Query,
        documents: &[Document],
        relevance: &[f64],
    ) -> Result<Vec<f64>, RankingV2Error> {
        if documents.len() > self.max_documents || relevance.len() != documents.len() {
            return Err(RankingV2Error::Backend);
        }
        let mut inputs = Vec::with_capacity(documents.len() + 1);
        inputs.push(query.normalized_query.clone());
        inputs.extend(documents.iter().map(document_text));
        let vectors = match self.backend.embed(&inputs).await {
            Ok(vectors) => vectors,
            Err(error) => {
                // Degrading is correct, degrading silently is not: a
                // misconfigured or unreachable backend would otherwise return
                // permanently worse rankings with no log, metric or signal.
                tracing::warn!(
                    target: "amatl::inference",
                    backend = self.backend.id(),
                    error = %error,
                    "embedding backend failed; falling back to lexical reranking"
                );
                return self.lexical.score(query, documents, relevance).await;
            }
        };
        let Some((query_vector, document_vectors)) = vectors.split_first() else {
            return Err(RankingV2Error::Backend);
        };
        if document_vectors.len() != documents.len() {
            return Err(RankingV2Error::Backend);
        }
        Ok(document_vectors
            .iter()
            .enumerate()
            .map(|(index, vector)| {
                let similarity = dot(query_vector, vector).clamp(0.0, 1.0);
                ((1.0 - self.prior_weight) * similarity + self.prior_weight * relevance[index])
                    .clamp(0.0, 1.0)
            })
            .collect())
    }
}

fn document_text(document: &Document) -> String {
    let title = document.title.as_deref().unwrap_or_default();
    let content = document.content.as_deref().unwrap_or_default();
    format!("{title} {content}")
}

fn bounded_text(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

fn terms(value: &str) -> Vec<String> {
    normalized_text(value)
        .split(|character: char| !character.is_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(str::to_owned)
        .collect()
}

fn feature_hash(value: &str) -> u64 {
    let digest = Sha256::digest(value.as_bytes());
    let mut bytes = [0_u8; 8];
    bytes.copy_from_slice(&digest[..8]);
    u64::from_be_bytes(bytes)
}

fn l2_normalized(vector: Vec<f32>) -> Vec<f32> {
    let norm = vector
        .iter()
        .map(|value| f64::from(*value) * f64::from(*value))
        .sum::<f64>()
        .sqrt();
    if norm == 0.0 {
        return vector;
    }
    vector
        .into_iter()
        .map(|value| (f64::from(value) / norm) as f32)
        .collect()
}

fn dot(left: &[f32], right: &[f32]) -> f64 {
    left.iter()
        .zip(right.iter())
        .map(|(left, right)| f64::from(*left) * f64::from(*right))
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{EgressPolicy, SecurityProfile};
    use crate::model::{DocumentStatus, FetchMethod, SCHEMA_VERSION};

    fn document(index: usize, title: &str, content: &str) -> Document {
        let url = url::Url::parse(&format!("https://inference.invalid/{index}")).unwrap();
        Document {
            schema_version: SCHEMA_VERSION.into(),
            search_result_id: index.to_string(),
            original_url: crate::OriginalUrl(url.clone()),
            canonical_url: crate::CanonicalUrl(url.clone()),
            final_url: crate::FinalUrl(url),
            content_hash: index.to_string(),
            fetch_method: FetchMethod::Http,
            extractor_used: Some("test".into()),
            content_type: Some("text/plain".into()),
            size: content.len() as u64,
            retrieved_at: "2026-08-12T00:00:00Z".into(),
            status: DocumentStatus::Enriched,
            content: Some(content.into()),
            title: Some(title.into()),
            author: None,
            published_at: None,
            metadata: BTreeMap::new(),
        }
    }

    fn corpus() -> Vec<Document> {
        vec![
            document(
                0,
                "Rust async runtime",
                "tokio is an async runtime for rust",
            ),
            document(1, "Bread recipe", "flour water salt and a long proof"),
        ]
    }

    fn runtime() -> InferenceRuntime {
        InferenceRuntime::local(&InferenceConfig::default()).unwrap()
    }

    #[tokio::test]
    async fn local_embeddings_are_deterministic_and_normalized() {
        let embedder = LocalHashingEmbedder::new(256, 20_000).unwrap();
        let inputs = vec!["rust async runtime".to_string()];
        let first = embedder.embed(&inputs).await.unwrap();
        assert_eq!(first, embedder.embed(&inputs).await.unwrap());
        assert_eq!(first[0].len(), 256);
        let norm = dot(&first[0], &first[0]);
        assert!((norm - 1.0).abs() < 1e-6, "{norm}");
    }

    #[tokio::test]
    async fn empty_input_yields_a_zero_vector_instead_of_failing() {
        let embedder = LocalHashingEmbedder::new(64, 128).unwrap();
        let vectors = embedder.embed(&["   ".to_string()]).await.unwrap();
        assert!(vectors[0].iter().all(|value| *value == 0.0));
    }

    #[tokio::test]
    async fn semantic_scorer_prefers_the_document_about_the_query() {
        let query = crate::query::parse_query("rust async runtime".into()).unwrap();
        let documents = corpus();
        let scores = runtime()
            .semantic_scorer()
            .score(&query, &documents)
            .await
            .unwrap();
        assert_eq!(scores.len(), 2);
        assert!(scores[0] > scores[1], "{scores:?}");
        assert!(scores.iter().all(|value| (0.0..=1.0).contains(value)));
    }

    #[tokio::test]
    async fn semantic_scorer_refuses_unbounded_document_batches() {
        let query = crate::query::parse_query("rust".into()).unwrap();
        let documents = corpus();
        let scorer =
            EmbeddingSemanticScorer::new(Arc::new(LocalHashingEmbedder::new(64, 512).unwrap()), 1);
        assert_eq!(
            scorer.score(&query, &documents).await,
            Err(RankingV2Error::Backend)
        );
    }

    #[tokio::test]
    async fn reranker_rewards_query_term_coverage_and_keeps_the_prior() {
        let query = crate::query::parse_query("rust async runtime".into()).unwrap();
        let documents = corpus();
        let reranker = runtime().reranker().unwrap();
        let scores = reranker
            .score(&query, &documents, &[0.5, 0.5])
            .await
            .unwrap();
        assert!(scores[0] > scores[1], "{scores:?}");
        assert_eq!(
            reranker.score(&query, &documents, &[0.5]).await,
            Err(RankingV2Error::Backend)
        );
    }

    #[test]
    fn policy_resolution_is_fail_closed_for_remote_inference() {
        let config = InferenceConfig::default();
        let mut policy = DataPolicyConfig {
            profile: SecurityProfile::Standard,
            egress: EgressPolicy::Governed,
            inference: InferenceMode::Disabled,
        };
        assert!(InferenceRuntime::from_policy(&policy, &config, None)
            .unwrap()
            .is_none());
        policy.inference = InferenceMode::LocalOnly;
        let local = InferenceRuntime::from_policy(&policy, &config, None)
            .unwrap()
            .unwrap();
        assert_eq!(local.backend_id(), LOCAL_EMBEDDING_BACKEND_ID);
        assert!(!local.is_remote());
        assert_eq!(local.version_key(), "local_hashing_v1@256");
        // Remote without an endpoint stays fail-closed.
        policy.inference = InferenceMode::RemoteExplicit;
        assert!(matches!(
            InferenceRuntime::from_policy(&policy, &config, None),
            Err(InferenceError::RemoteBackendUnavailable)
        ));
        // An isolated profile cannot reach a remote backend even when one is
        // fully configured.
        let remote_config = InferenceConfig {
            remote_endpoint: Some("https://embeddings.invalid/v1/embeddings".into()),
            remote_model: Some("test-model".into()),
            ..InferenceConfig::default()
        };
        policy.profile = SecurityProfile::Isolated;
        assert!(matches!(
            InferenceRuntime::from_policy(&policy, &remote_config, None),
            Err(InferenceError::RemoteBackendUnavailable)
        ));
    }

    /// One recorded outbound request: URL, headers and body.
    type SeenRequest = (String, Vec<(String, String)>, Vec<u8>);

    /// Records what the backend sent and replies with a canned payload.
    struct FakeTransport {
        response: std::sync::Mutex<Vec<crate::providers::HttpResponse>>,
        seen: std::sync::Mutex<Vec<SeenRequest>>,
    }

    impl FakeTransport {
        fn new(responses: Vec<crate::providers::HttpResponse>) -> Arc<Self> {
            Arc::new(Self {
                response: std::sync::Mutex::new(responses),
                seen: std::sync::Mutex::new(vec![]),
            })
        }
    }

    #[async_trait]
    impl HttpTransport for FakeTransport {
        async fn execute(
            &self,
            request: HttpRequest,
        ) -> Result<crate::providers::HttpResponse, String> {
            self.seen.lock().unwrap().push((
                request.url.to_string(),
                request.headers.clone(),
                request.body.clone().unwrap_or_default(),
            ));
            let mut responses = self.response.lock().unwrap();
            if responses.is_empty() {
                return Err("no canned response".into());
            }
            Ok(responses.remove(0))
        }
    }

    fn embeddings_response(vectors: &[Vec<f32>]) -> crate::providers::HttpResponse {
        let data = vectors
            .iter()
            .map(|vector| serde_json::json!({ "embedding": vector }))
            .collect::<Vec<_>>();
        crate::providers::HttpResponse {
            status: 200,
            headers: BTreeMap::new(),
            body: serde_json::to_vec(&serde_json::json!({ "data": data })).unwrap(),
        }
    }

    fn remote_config() -> InferenceConfig {
        InferenceConfig {
            embedding_dimensions: 32,
            remote_endpoint: Some("https://embeddings.invalid/v1/embeddings".into()),
            remote_model: Some("text-embeddings".into()),
            remote_max_batch: 2,
            ..InferenceConfig::default()
        }
    }

    #[tokio::test]
    async fn remote_backend_batches_bounded_requests_and_normalizes_vectors() {
        let vector = vec![3.0_f32; 32];
        let transport = FakeTransport::new(vec![
            embeddings_response(&[vector.clone(), vector.clone()]),
            embeddings_response(std::slice::from_ref(&vector)),
        ]);
        let backend = RemoteEmbeddingBackend::new(&remote_config(), transport.clone()).unwrap();
        assert!(backend.is_remote());
        let inputs = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let vectors = backend.embed(&inputs).await.unwrap();
        assert_eq!(vectors.len(), 3);
        for vector in &vectors {
            assert_eq!(vector.len(), 32);
            assert!((dot(vector, vector) - 1.0).abs() < 1e-6);
        }
        let seen = transport.seen.lock().unwrap();
        assert_eq!(seen.len(), 2, "max_batch must split the inputs");
        assert!(seen[0].0.starts_with("https://embeddings.invalid/"));
        assert!(seen[0]
            .1
            .iter()
            .any(|(name, value)| name == "content-type" && value == "application/json"));
        let body: serde_json::Value = serde_json::from_slice(&seen[0].2).unwrap();
        assert_eq!(body["model"], "text-embeddings");
        assert_eq!(body["input"].as_array().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn remote_backend_sends_the_credential_only_as_a_bearer_header() {
        let transport = FakeTransport::new(vec![embeddings_response(&[vec![1.0_f32; 32]])]);
        let backend = RemoteEmbeddingBackend::with_credential(
            &remote_config(),
            transport.clone(),
            Some("secret-embedding-key".into()),
        )
        .unwrap();
        backend.embed(&["a".to_string()]).await.unwrap();
        let seen = transport.seen.lock().unwrap();
        assert!(seen[0].1.iter().any(
            |(name, value)| name == "authorization" && value.ends_with("secret-embedding-key")
        ));
        assert!(!seen[0].0.contains("secret-embedding-key"));
        assert!(!String::from_utf8_lossy(&seen[0].2).contains("secret-embedding-key"));
    }

    #[tokio::test]
    async fn remote_backend_rejects_responses_that_break_the_contract() {
        let wrong_width = FakeTransport::new(vec![embeddings_response(&[vec![1.0_f32; 8]])]);
        let backend = RemoteEmbeddingBackend::new(&remote_config(), wrong_width).unwrap();
        assert_eq!(
            backend.embed(&["a".into()]).await,
            Err(InferenceError::RemoteResponseInvalid)
        );

        let wrong_count = FakeTransport::new(vec![embeddings_response(&[])]);
        let backend = RemoteEmbeddingBackend::new(&remote_config(), wrong_count).unwrap();
        assert_eq!(
            backend.embed(&["a".into()]).await,
            Err(InferenceError::RemoteResponseInvalid)
        );

        let refused = FakeTransport::new(vec![crate::providers::HttpResponse {
            status: 401,
            headers: BTreeMap::new(),
            body: b"{}".to_vec(),
        }]);
        let backend = RemoteEmbeddingBackend::new(&remote_config(), refused).unwrap();
        assert_eq!(
            backend.embed(&["a".into()]).await,
            Err(InferenceError::RemoteRequestFailed)
        );
    }

    #[tokio::test]
    async fn remote_runtime_reports_its_own_vector_space() {
        let transport = FakeTransport::new(vec![]);
        let runtime = InferenceRuntime::remote(&remote_config(), transport).unwrap();
        assert!(runtime.is_remote());
        assert_eq!(runtime.version_key(), "remote_embeddings_v1@32");
        assert_ne!(runtime.version_key(), self::runtime().version_key());
    }

    #[test]
    fn remote_backend_requires_a_declared_endpoint_and_model() {
        let transport = FakeTransport::new(vec![]);
        assert!(matches!(
            RemoteEmbeddingBackend::new(&InferenceConfig::default(), transport.clone()),
            Err(InferenceError::RemoteBackendUnavailable)
        ));
        let no_model = InferenceConfig {
            remote_model: None,
            ..remote_config()
        };
        assert!(matches!(
            RemoteEmbeddingBackend::new(&no_model, transport.clone()),
            Err(InferenceError::RemoteBackendUnavailable)
        ));
        let missing_credential = InferenceConfig {
            remote_credential_env: Some("AMATL_TEST_ABSENT_EMBEDDING_KEY".into()),
            ..remote_config()
        };
        assert!(matches!(
            RemoteEmbeddingBackend::new(&missing_credential, transport),
            Err(InferenceError::RemoteBackendUnavailable)
        ));
    }

    #[test]
    fn unknown_backends_and_invalid_limits_are_rejected() {
        let unknown = InferenceConfig {
            backend: "hosted-model".into(),
            ..InferenceConfig::default()
        };
        assert!(matches!(
            InferenceRuntime::local(&unknown),
            Err(InferenceError::UnknownBackend(name)) if name == "hosted-model"
        ));
        let invalid = InferenceConfig {
            embedding_dimensions: 8,
            ..InferenceConfig::default()
        };
        assert!(matches!(
            InferenceRuntime::local(&invalid),
            Err(InferenceError::InvalidLimit)
        ));
    }

    fn write_model(path: &std::path::Path, dimensions: usize) {
        let mut content = String::from("# test word vectors\n");
        for (token, values) in [
            ("rust", vec![1.0, 0.0, 0.0]),
            ("async", vec![0.9, 0.1, 0.0]),
            ("runtime", vec![0.8, 0.2, 0.0]),
            ("bread", vec![0.0, 1.0, 0.0]),
            ("recipe", vec![0.1, 0.9, 0.0]),
        ] {
            content.push_str(token);
            for index in 0..dimensions {
                content.push(' ');
                content.push_str(&values.get(index).copied().unwrap_or(0.0).to_string());
            }
            content.push('\n');
        }
        std::fs::write(path, content).unwrap();
    }

    #[test]
    fn local_model_backend_loads_and_embeds_from_a_model_file() {
        let dir = std::env::temp_dir().join(format!("amatl-model-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("vectors.txt");
        write_model(&path, 64);
        let embedder = LocalModelEmbedder::load(&path, 64, 512).unwrap();
        let vectors = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(embedder.embed(&["rust async runtime".into(), "bread recipe".into()]))
            .unwrap();
        assert_eq!(vectors.len(), 2);
        for vector in &vectors {
            let norm = dot(vector, vector);
            assert!((norm - 1.0).abs() < 1e-6, "{norm}");
        }
        // The rust cluster is closer to the rust query than the bread cluster.
        let query = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(embedder.embed(&["rust async".into()]))
            .unwrap();
        assert!(dot(&query[0], &vectors[0]) > dot(&query[0], &vectors[1]));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn local_model_backend_fails_closed_on_missing_or_malformed_models() {
        let missing = std::path::Path::new("/definitely/not/a/model.txt");
        assert!(matches!(
            LocalModelEmbedder::load(missing, 64, 512),
            Err(InferenceError::ModelUnavailable)
        ));
        let dir = std::env::temp_dir().join(format!("amatl-badmodel-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("bad.txt");
        std::fs::write(&path, "token not_a_number\n").unwrap();
        assert!(matches!(
            LocalModelEmbedder::load(&path, 64, 512),
            Err(InferenceError::ModelUnavailable)
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn embedding_cache_persists_and_namespaces_by_vector_space() {
        let dir = std::env::temp_dir().join(format!("amatl-cache-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("cache.json");
        let mut cache = EmbeddingCache::load(path.clone(), "local_hashing_v1@64".into(), 16);
        cache.insert("hello world", vec![1.0, 2.0, 3.0]);
        cache.save();
        // A fresh load from disk sees the persisted entry.
        let mut reloaded = EmbeddingCache::load(path.clone(), "local_hashing_v1@64".into(), 16);
        assert_eq!(reloaded.get("hello world"), Some(vec![1.0, 2.0, 3.0]));
        // A different namespace never reuses the artifact.
        let mut other = EmbeddingCache::load(path.clone(), "local_model_v1@64".into(), 16);
        assert_eq!(other.get("hello world"), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn embedding_reranker_uses_semantic_similarity_and_degrades_to_lexical() {
        let query = crate::query::parse_query("rust async runtime".into()).unwrap();
        let documents = corpus();
        // Built directly: the default runtime deliberately does *not* select
        // this reranker, because the default backend is a feature hash rather
        // than a model (see `InferenceRuntime::reranker`).
        let reranker = EmbeddingReranker::new(runtime().embedding_backend(), 64, 0.5).unwrap();
        assert_eq!(reranker.name(), "embedding_semantic_v1");
        let scores = reranker
            .score(&query, &documents, &[0.5, 0.5])
            .await
            .unwrap();
        assert!(scores[0] > scores[1], "{scores:?}");
        assert!(scores.iter().all(|value| (0.0..=1.0).contains(value)));
        // A backend that always fails degrades to the lexical reranker.
        struct Broken;
        #[async_trait]
        impl EmbeddingBackend for Broken {
            fn id(&self) -> &str {
                "broken"
            }
            fn dimensions(&self) -> usize {
                64
            }
            async fn embed(&self, _: &[String]) -> Result<Vec<Vec<f32>>, InferenceError> {
                Err(InferenceError::ModelUnavailable)
            }
        }
        let degraded = EmbeddingReranker::new(Arc::new(Broken), 64, 0.5).unwrap();
        let scores = degraded
            .score(&query, &documents, &[0.5, 0.5])
            .await
            .unwrap();
        assert!(scores[0] > scores[1], "{scores:?}");
    }

    #[test]
    fn local_model_backend_is_selectable_through_the_runtime() {
        let dir = std::env::temp_dir().join(format!("amatl-runtime-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("vectors.txt");
        write_model(&path, 64);
        assert!(path.exists(), "model file missing at {path:?}");
        let config = InferenceConfig {
            backend: LOCAL_MODEL_BACKEND_ID.into(),
            local_model_path: Some(path.to_string_lossy().into_owned()),
            embedding_dimensions: 64,
            ..InferenceConfig::default()
        };
        let runtime = InferenceRuntime::local(&config).unwrap();
        assert_eq!(runtime.backend_id(), LOCAL_MODEL_BACKEND_ID);
        assert_eq!(runtime.version_key(), "local_model_v1@64");
        // Without a model path the model backend fails closed.
        let no_model = InferenceConfig {
            backend: LOCAL_MODEL_BACKEND_ID.into(),
            local_model_path: None,
            ..InferenceConfig::default()
        };
        assert!(matches!(
            InferenceRuntime::local(&no_model),
            Err(InferenceError::ModelUnavailable)
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
