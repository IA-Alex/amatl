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
        if config.backend != LOCAL_EMBEDDING_BACKEND_ID {
            return Err(InferenceError::UnknownBackend(config.backend.clone()));
        }
        if config.max_documents == 0 {
            return Err(InferenceError::InvalidLimit);
        }
        let backend =
            LocalHashingEmbedder::new(config.embedding_dimensions, config.max_input_chars)?;
        Ok(Self {
            backend: Arc::new(backend),
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

    pub fn reranker(&self) -> Result<Arc<dyn DeepReranker>, InferenceError> {
        LexicalCoverageReranker::new(self.max_documents, self.reranker_prior_weight)
            .map(|value| Arc::new(value) as Arc<dyn DeepReranker>)
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
}
