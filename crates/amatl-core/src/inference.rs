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
//! Remote inference has no implementation on purpose: `remote_explicit`
//! resolves to [`InferenceError::RemoteBackendUnavailable`] so no surface can
//! silently send corpus text to a third party.

use crate::config::{DataPolicyConfig, InferenceConfig, InferenceMode};
use crate::model::{Document, Query};
use crate::ranking_v2::{DeepReranker, RankingV2Error, SemanticScorer};
use crate::text::normalized_text;
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
}

/// Contract every embedding backend must honor.
///
/// Implementors are free to be remote or model-backed, but a backend handed to
/// AMATL under `local_only` must not perform network or filesystem access.
pub trait EmbeddingBackend: Send + Sync {
    /// Stable backend identifier recorded in ranking explanations and logs.
    fn id(&self) -> &str;
    /// Width of every produced vector.
    fn dimensions(&self) -> usize;
    /// Embed inputs in order, returning one L2-normalized vector per input.
    fn embed(&self, inputs: &[String]) -> Result<Vec<Vec<f32>>, InferenceError>;
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

impl EmbeddingBackend for LocalHashingEmbedder {
    fn id(&self) -> &str {
        LOCAL_EMBEDDING_BACKEND_ID
    }

    fn dimensions(&self) -> usize {
        self.dimensions
    }

    fn embed(&self, inputs: &[String]) -> Result<Vec<Vec<f32>>, InferenceError> {
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

impl SemanticScorer for EmbeddingSemanticScorer {
    fn name(&self) -> &str {
        self.backend.id()
    }

    fn score(&self, query: &Query, documents: &[Document]) -> Result<Vec<f64>, RankingV2Error> {
        if documents.len() > self.max_documents {
            return Err(RankingV2Error::Backend);
        }
        let mut inputs = Vec::with_capacity(documents.len() + 1);
        inputs.push(query.normalized_query.clone());
        inputs.extend(documents.iter().map(document_text));
        let vectors = self
            .backend
            .embed(&inputs)
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

impl DeepReranker for LexicalCoverageReranker {
    fn name(&self) -> &str {
        LOCAL_RERANKER_ID
    }

    fn score(
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
    ) -> Result<Option<Self>, InferenceError> {
        match policy.inference {
            InferenceMode::Disabled => Ok(None),
            InferenceMode::LocalOnly => Self::local(config).map(Some),
            InferenceMode::RemoteExplicit => Err(InferenceError::RemoteBackendUnavailable),
        }
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

    #[test]
    fn local_embeddings_are_deterministic_and_normalized() {
        let embedder = LocalHashingEmbedder::new(256, 20_000).unwrap();
        let inputs = vec!["rust async runtime".to_string()];
        let first = embedder.embed(&inputs).unwrap();
        assert_eq!(first, embedder.embed(&inputs).unwrap());
        assert_eq!(first[0].len(), 256);
        let norm = dot(&first[0], &first[0]);
        assert!((norm - 1.0).abs() < 1e-6, "{norm}");
    }

    #[test]
    fn empty_input_yields_a_zero_vector_instead_of_failing() {
        let embedder = LocalHashingEmbedder::new(64, 128).unwrap();
        let vectors = embedder.embed(&["   ".to_string()]).unwrap();
        assert!(vectors[0].iter().all(|value| *value == 0.0));
    }

    #[test]
    fn semantic_scorer_prefers_the_document_about_the_query() {
        let query = crate::query::parse_query("rust async runtime".into()).unwrap();
        let documents = corpus();
        let scores = runtime()
            .semantic_scorer()
            .score(&query, &documents)
            .unwrap();
        assert_eq!(scores.len(), 2);
        assert!(scores[0] > scores[1], "{scores:?}");
        assert!(scores.iter().all(|value| (0.0..=1.0).contains(value)));
    }

    #[test]
    fn semantic_scorer_refuses_unbounded_document_batches() {
        let query = crate::query::parse_query("rust".into()).unwrap();
        let documents = corpus();
        let scorer =
            EmbeddingSemanticScorer::new(Arc::new(LocalHashingEmbedder::new(64, 512).unwrap()), 1);
        assert_eq!(
            scorer.score(&query, &documents),
            Err(RankingV2Error::Backend)
        );
    }

    #[test]
    fn reranker_rewards_query_term_coverage_and_keeps_the_prior() {
        let query = crate::query::parse_query("rust async runtime".into()).unwrap();
        let documents = corpus();
        let reranker = runtime().reranker().unwrap();
        let scores = reranker.score(&query, &documents, &[0.5, 0.5]).unwrap();
        assert!(scores[0] > scores[1], "{scores:?}");
        assert_eq!(
            reranker.score(&query, &documents, &[0.5]),
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
        assert!(InferenceRuntime::from_policy(&policy, &config)
            .unwrap()
            .is_none());
        policy.inference = InferenceMode::LocalOnly;
        assert_eq!(
            InferenceRuntime::from_policy(&policy, &config)
                .unwrap()
                .unwrap()
                .backend_id(),
            LOCAL_EMBEDDING_BACKEND_ID
        );
        policy.inference = InferenceMode::RemoteExplicit;
        assert!(matches!(
            InferenceRuntime::from_policy(&policy, &config),
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
