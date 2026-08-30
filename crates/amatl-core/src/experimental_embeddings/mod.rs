//! STEP 4D — EXPERIMENTAL local-embedding integration.
//!
//! **This module is compiled only under `--features experimental-local-embeddings`
//! and is never reachable from the default search path.** STEP 4C established
//! the typed seam here; STEP 4D moves the *real* pure-Rust Candle backend into
//! the workspace behind the same feature (see [`candle`]), adds the optional
//! model-package config with pinned hashes ([`model_config`]), and the
//! optimized bounded-K semantic-evaluation pipeline ([`pipeline`]).
//!
//! The seam still attaches without any production relevance, routing, ranking,
//! or telemetry behaviour changing. The real backend is additionally gated on a
//! valid model package being present; absent/corrupt → bounded-semantic
//! fallback.
//!
//! ## Hard invariants
//!
//! * The production default is **unchanged**. [`crate::relevance::assess_relevance`]
//!   and [`crate::relevance::assess_result`] do not call anything here.
//! * Lexical + bounded-semantic relevance ([`crate::relevance_semantics`]) stays
//!   authoritative. An embedding signal here can only ever be an *advisory
//!   paraphrase-recall booster*, subordinate to negative / entity evidence.
//! * Embedding failure (missing model, corrupt weights, hash mismatch, backend
//!   error, empty input) **must fall back cleanly** to the bounded-semantic
//!   outcome. No panic, no error surfaced to the caller, no provider
//!   suppression, no routing mutation, no telemetry mutation.
//! * No network. A backend that would need to download a model must instead
//!   return [`EmbeddingUnavailable::ModelMissing`].
//!
//! ## What lives here
//!
//! * [`EmbeddingBackend`] — the trait an experimental backend implements. The
//!   real Candle implementation stays in the excluded experiment crate; only a
//!   deterministic [`FixtureBackend`] ships here, for the latency harness and
//!   the failure-mode tests.
//! * [`SemanticRescore`] — the advisory output. It carries only a bounded
//!   cosine-derived hint and the fallback flag; it deliberately has **no** way
//!   to express "this result is RELEVANT".
//! * [`evaluate_with_embeddings`] — the seam. Given a bounded-semantic
//!   [`SemanticAssessment`] and a backend, returns a [`SemanticRescore`],
//!   falling back to a neutral result on any backend failure.

use crate::model::Query;
use crate::relevance_semantics::SemanticAssessment;

pub mod candle;
pub mod model_config;
pub mod pipeline;

pub use candle::CandleBackend;
pub use model_config::{ModelHashes, ModelPackage, ModelPackageError};
pub use pipeline::{
    ExperimentalSemanticConfig, SemanticEvaluation, SemanticEvaluator, DEFAULT_EXPERIMENTAL_K,
};

/// Why an embedding backend could not produce a vector. Every variant is a
/// clean, non-panicking outcome that the seam turns into a neutral fallback.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EmbeddingUnavailable {
    /// Model file absent at the configured path.
    ModelMissing,
    /// Model / tokenizer / config file present but unreadable or malformed.
    ModelCorrupt,
    /// Pinned sha256 of the model or tokenizer did not match the file on disk.
    HashMismatch,
    /// Tokenizer file absent.
    TokenizerMissing,
    /// Config file absent.
    ConfigMissing,
    /// Input was empty after normalisation — nothing to embed.
    EmptyInput,
    /// The backend ran but failed (shape error, NaN, allocation, …).
    BackendError(String),
}

impl std::fmt::Display for EmbeddingUnavailable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ModelMissing => write!(f, "embedding model missing"),
            Self::ModelCorrupt => write!(f, "embedding model corrupt"),
            Self::HashMismatch => write!(f, "embedding model hash mismatch"),
            Self::TokenizerMissing => write!(f, "embedding tokenizer missing"),
            Self::ConfigMissing => write!(f, "embedding config missing"),
            Self::EmptyInput => write!(f, "empty embedding input"),
            Self::BackendError(msg) => write!(f, "embedding backend error: {msg}"),
        }
    }
}

/// An experimental local sentence-embedding backend.
///
/// Implementations must be deterministic, fully offline, and must return an
/// [`EmbeddingUnavailable`] (never panic, never block on I/O) when they cannot
/// produce a vector.
pub trait EmbeddingBackend: Send + Sync {
    /// Embedding dimension (e.g. 384 for `bge-small-en-v1.5`).
    fn dimension(&self) -> usize;

    /// Embed a search query. The implementation owns any instruction prefix.
    fn embed_query(&self, text: &str) -> Result<Vec<f32>, EmbeddingUnavailable>;

    /// Embed a document (title + snippet, already concatenated by the caller).
    fn embed_document(&self, text: &str) -> Result<Vec<f32>, EmbeddingUnavailable>;
}

/// Cosine similarity of two equal-length vectors. Returns `0.0` on length
/// mismatch or a zero vector — never `NaN`.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let mut dot = 0.0f32;
    let mut na = 0.0f32;
    let mut nb = 0.0f32;
    for (x, y) in a.iter().zip(b.iter()) {
        dot += x * y;
        na += x * x;
        nb += y * y;
    }
    if na <= f32::EPSILON || nb <= f32::EPSILON {
        return 0.0;
    }
    (dot / (na.sqrt() * nb.sqrt())).clamp(-1.0, 1.0)
}

/// Advisory output of the experimental seam.
///
/// Deliberately minimal: a bounded hint and whether we fell back. There is no
/// field that can promote a result to RELEVANT — the bounded-semantic layer
/// remains the only thing that can do that.
#[derive(Debug, Clone, PartialEq)]
pub struct SemanticRescore {
    /// Cosine similarity of the query and document embeddings, in `[-1, 1]`.
    /// `0.0` when `used_fallback` is set.
    pub cosine: f32,
    /// `true` when the embedding backend was unavailable and the caller should
    /// use the bounded-semantic outcome unchanged.
    pub used_fallback: bool,
    /// Present only when `used_fallback` is set: the reason, for diagnostics.
    pub fallback_reason: Option<EmbeddingUnavailable>,
    /// Echo of [`SemanticAssessment::is_strong_rescue`] — the bounded-semantic
    /// paraphrase signal this advisory sits *next to*, never overrides.
    pub bounded_strong_rescue: bool,
}

impl SemanticRescore {
    /// Neutral fallback: no embedding contribution, bounded-semantic authoritative.
    fn fallback(reason: EmbeddingUnavailable, bounded: &SemanticAssessment) -> Self {
        Self {
            cosine: 0.0,
            used_fallback: true,
            fallback_reason: Some(reason),
            bounded_strong_rescue: bounded.is_strong_rescue(),
        }
    }

    /// Whether the embedding cosine *agrees* the result is a plausible
    /// paraphrase (advisory only). Threshold reused from the STEP 4A/4B ONNX
    /// sweep (`t_high = 0.72`); **not** re-tuned against any hold-out here.
    pub fn embedding_supports_paraphrase(&self) -> bool {
        !self.used_fallback && self.cosine >= 0.72
    }
}

/// The experimental seam.
///
/// Computes an advisory [`SemanticRescore`] for one (query, result-text) pair.
/// On *any* backend failure it returns [`SemanticRescore::fallback`] carrying
/// the bounded-semantic assessment unchanged. Never panics, never does I/O
/// itself, never mutates its inputs.
pub fn evaluate_with_embeddings(
    query: &Query,
    result_text: &str,
    bounded: &SemanticAssessment,
    backend: &dyn EmbeddingBackend,
) -> SemanticRescore {
    let q = query.normalized_query.trim();
    if q.is_empty() {
        return SemanticRescore::fallback(EmbeddingUnavailable::EmptyInput, bounded);
    }
    if result_text.trim().is_empty() {
        return SemanticRescore::fallback(EmbeddingUnavailable::EmptyInput, bounded);
    }

    let qv = match backend.embed_query(q) {
        Ok(v) => v,
        Err(e) => return SemanticRescore::fallback(e, bounded),
    };
    let dv = match backend.embed_document(result_text) {
        Ok(v) => v,
        Err(e) => return SemanticRescore::fallback(e, bounded),
    };

    let cosine = cosine_similarity(&qv, &dv);
    if !cosine.is_finite() {
        return SemanticRescore::fallback(
            EmbeddingUnavailable::BackendError("non-finite cosine".into()),
            bounded,
        );
    }

    SemanticRescore {
        cosine,
        used_fallback: false,
        fallback_reason: None,
        bounded_strong_rescue: bounded.is_strong_rescue(),
    }
}

/// Deterministic, dependency-free backend for the STEP 4C latency harness and
/// the failure-mode tests. **Not** a semantic model — it hashes token shingles
/// into a fixed-dimension vector so that identical text embeds identically and
/// overlapping text has higher cosine, with a fixed per-call CPU cost that
/// stands in for real inference in the E2E benchmark.
///
/// A real deployment would use the Candle `bge-small-en-v1.5` backend from the
/// experiment crate; this exists so the seam can be exercised in-workspace
/// without shipping ~200 ML crates in the production lockfile.
pub struct FixtureBackend {
    dim: usize,
    /// Simulated per-embed CPU cost, in units of hash rounds. Set from the
    /// STEP 4B measured Candle numbers by the harness.
    work_rounds: u32,
    /// When set, every call returns this error — drives the failure-mode tests.
    forced_error: Option<EmbeddingUnavailable>,
}

impl Default for FixtureBackend {
    fn default() -> Self {
        Self {
            dim: 384,
            work_rounds: 0,
            forced_error: None,
        }
    }
}

impl FixtureBackend {
    /// A backend that always fails with `reason` — for the failure-mode tests.
    pub fn failing(reason: EmbeddingUnavailable) -> Self {
        Self {
            forced_error: Some(reason),
            ..Self::default()
        }
    }

    /// Add simulated CPU cost per embed call (hash rounds). Used by the latency
    /// harness to reproduce the STEP 4B per-embed timing.
    pub fn with_work_rounds(mut self, rounds: u32) -> Self {
        self.work_rounds = rounds;
        self
    }

    fn embed(&self, text: &str) -> Result<Vec<f32>, EmbeddingUnavailable> {
        if let Some(e) = &self.forced_error {
            return Err(e.clone());
        }
        let normalized = crate::text::normalized_text(text);
        if normalized.trim().is_empty() {
            return Err(EmbeddingUnavailable::EmptyInput);
        }

        let mut v = vec![0.0f32; self.dim];
        let toks: Vec<&str> = normalized.split_whitespace().collect();
        for w in toks.windows(2).chain(toks.windows(1)) {
            let joined = w.join(" ");
            let h = fnv1a(joined.as_bytes());
            let idx = (h as usize) % self.dim;
            let sign = if h & 1 == 0 { 1.0 } else { -1.0 };
            v[idx] += sign;
        }

        // Simulated inference cost: repeatedly re-hash the *actual input bytes*
        // together with a running accumulator and fold the result back into the
        // output vector, so the optimiser cannot hoist or elide the loop (its
        // work depends on `text` and its result is observable in `v`).
        let bytes = normalized.as_bytes();
        let mut acc = fnv1a(bytes);
        for r in 0..self.work_rounds {
            acc = fnv1a(&acc.to_le_bytes()) ^ (r as u64);
            acc = acc.wrapping_add(bytes[(acc as usize) % bytes.len()] as u64);
        }
        let sink_idx = (acc as usize) % self.dim;
        v[sink_idx] += (acc & 0xff) as f32 * f32::MIN_POSITIVE;

        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm > f32::EPSILON {
            for x in &mut v {
                *x /= norm;
            }
        }
        Ok(v)
    }
}

impl EmbeddingBackend for FixtureBackend {
    fn dimension(&self) -> usize {
        self.dim
    }
    fn embed_query(&self, text: &str) -> Result<Vec<f32>, EmbeddingUnavailable> {
        self.embed(text)
    }
    fn embed_document(&self, text: &str) -> Result<Vec<f32>, EmbeddingUnavailable> {
        self.embed(text)
    }
}

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h = 0xcbf29ce484222325u64;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Query;
    use crate::parse_query;

    fn q(s: &str) -> Query {
        parse_query(s.to_string()).unwrap()
    }

    /// A `Query` whose `normalized_query` is empty — only reachable by
    /// constructing the struct directly (`parse_query` rejects empty input).
    fn empty_normalized_query() -> Query {
        let mut base = parse_query("placeholder".to_string()).unwrap();
        base.normalized_query.clear();
        base.raw_query.clear();
        base
    }

    #[test]
    fn identical_text_embeds_identically_and_cosine_is_one() {
        let b = FixtureBackend::default();
        let a = b.embed_query("kubernetes networking model").unwrap();
        let c = b.embed_document("kubernetes networking model").unwrap();
        assert!((cosine_similarity(&a, &c) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn empty_query_falls_back_cleanly() {
        let b = FixtureBackend::default();
        let bounded = SemanticAssessment::default();
        let r = evaluate_with_embeddings(&empty_normalized_query(), "some text", &bounded, &b);
        assert!(r.used_fallback);
        assert_eq!(r.cosine, 0.0);
        assert_eq!(r.fallback_reason, Some(EmbeddingUnavailable::EmptyInput));
    }

    #[test]
    fn empty_result_text_falls_back_cleanly() {
        let b = FixtureBackend::default();
        let bounded = SemanticAssessment::default();
        let r = evaluate_with_embeddings(&q("docs"), "   ", &bounded, &b);
        assert!(r.used_fallback);
    }

    #[test]
    fn backend_failure_falls_back_and_never_panics() {
        let bounded = SemanticAssessment::default();
        for reason in [
            EmbeddingUnavailable::ModelMissing,
            EmbeddingUnavailable::ModelCorrupt,
            EmbeddingUnavailable::HashMismatch,
            EmbeddingUnavailable::TokenizerMissing,
            EmbeddingUnavailable::ConfigMissing,
            EmbeddingUnavailable::BackendError("boom".into()),
        ] {
            let b = FixtureBackend::failing(reason.clone());
            let r = evaluate_with_embeddings(&q("kubernetes"), "k8s guide", &bounded, &b);
            assert!(r.used_fallback);
            assert_eq!(r.fallback_reason, Some(reason));
            assert_eq!(r.cosine, 0.0);
        }
    }

    #[test]
    fn non_ascii_and_long_input_do_not_panic() {
        let b = FixtureBackend::default();
        let bounded = SemanticAssessment::default();
        let _ = evaluate_with_embeddings(&q("café schrödinger 日本語 —"), "λ ω", &bounded, &b);
        let long = "word ".repeat(50_000);
        let r = evaluate_with_embeddings(&q("word"), &long, &bounded, &b);
        assert!(!r.used_fallback);
        assert!(r.cosine.is_finite());
    }

    #[test]
    fn rescore_cannot_express_relevant() {
        // Type-level check: SemanticRescore exposes only advisory accessors.
        let r = SemanticRescore {
            cosine: 0.99,
            used_fallback: false,
            fallback_reason: None,
            bounded_strong_rescue: false,
        };
        assert!(r.embedding_supports_paraphrase());
        // There is intentionally no `is_relevant` / `classification` on the type.
    }

    #[test]
    fn deterministic_across_calls() {
        let b = FixtureBackend::default();
        let v1 = b.embed_query("repeatable input text").unwrap();
        let v2 = b.embed_query("repeatable input text").unwrap();
        assert_eq!(v1, v2);
    }
}
