//! STEP 4D — optimized experimental semantic-evaluation pipeline.
//!
//! The naive STEP 4C path (embed the query once *per result*, embed every
//! result, semantically score all of them) is not acceptable. This module
//! implements the required execution architecture:
//!
//! * **Query embedding — exactly once per search.** [`SemanticEvaluator::new`]
//!   embeds the query a single time; every candidate reuses that vector. The
//!   [`SemanticEvaluation::query_embed_count`] invariant is asserted by tests.
//! * **Document embedding — batched.** The whole bounded candidate set goes
//!   through one [`CandleBackend::embed_documents`] forward pass.
//! * **Bounded candidate set.** Only results in the *ambiguous relevance band*
//!   (primarily [`RelevanceClassification::PossiblyRelevant`]) are considered,
//!   capped at [`ExperimentalSemanticConfig::k`] (default
//!   [`DEFAULT_EXPERIMENTAL_K`] = 8). High-confidence results and results with
//!   blocking negative/entity evidence are **never** embedded.
//! * **Advisory only.** The output is a [`SemanticRescore`] per candidate plus
//!   an optional *suggested* classification. Precedence is fixed:
//!
//!   ```text
//!   explicit negative / entity evidence  >  semantic embedding evidence  >  lexical corroboration
//!   ```
//!
//!   Embedding can nudge a `PossiblyRelevant` result up to a *suggested*
//!   `Relevant` only when there is **no blocking contradiction** and the
//!   bounded-semantic layer already offers corroboration. It can never promote
//!   a contradiction case, and it is never a sovereign classifier — the
//!   production `assess_result` output is passed through untouched; the
//!   suggestion is a separate, clearly-labelled field.
//!
//! Nothing here is wired into routing, ranking, telemetry, or provider
//! selection. When the feature is off the whole module is uncompiled.

use crate::model::{DeduplicatedResult, Query, RelevanceClassification, ResultRelevanceAssessment};
use crate::relevance::{assess_result, RelevanceThresholds};
use crate::relevance_semantics::assess_semantics;

use super::candle::CandleBackend;
use super::model_config::ModelPackage;
use super::{cosine_similarity, EmbeddingBackend, EmbeddingUnavailable, SemanticRescore};

/// Default bounded candidate count `K` for the experimental semantic pass.
/// Small on purpose (spec §3: "5–10"). Provisional — frozen from the STEP 4A/4B
/// sweep, **not** re-tuned against any hold-out.
pub const DEFAULT_EXPERIMENTAL_K: usize = 8;

/// Similarity at or above which the embedding *agrees* a `PossiblyRelevant`
/// result is a plausible paraphrase. Reused verbatim from the STEP 4A/4B ONNX
/// sweep (`t_high = 0.72`); **provisional**, not re-tuned here (spec §10).
pub const PROVISIONAL_PARAPHRASE_THRESHOLD: f32 = 0.72;

/// Tuning knobs for the experimental pass. All provisional / frozen.
#[derive(Debug, Clone, Copy)]
pub struct ExperimentalSemanticConfig {
    /// Max results semantically evaluated per search.
    pub k: usize,
    /// Cosine threshold for paraphrase agreement.
    pub paraphrase_threshold: f32,
    /// Also evaluate `Unknown` results (have some text but under-judged).
    /// Off by default — `PossiblyRelevant` is the primary ambiguous band.
    pub include_unknown: bool,
}

impl Default for ExperimentalSemanticConfig {
    fn default() -> Self {
        Self {
            k: DEFAULT_EXPERIMENTAL_K,
            paraphrase_threshold: PROVISIONAL_PARAPHRASE_THRESHOLD,
            include_unknown: false,
        }
    }
}

/// Per-candidate outcome of the experimental pass.
#[derive(Debug, Clone)]
pub struct SemanticEvaluation {
    /// Index into the original `results` slice.
    pub result_index: usize,
    /// The production assessment, passed through **unchanged**.
    pub production: ResultRelevanceAssessment,
    /// The advisory embedding rescore (fallback-safe).
    pub rescore: SemanticRescore,
    /// A *suggested* classification, set only when the embedding advisory would
    /// change the production label under the fixed precedence rules. `None`
    /// means "no change suggested". This is never applied automatically.
    pub suggested: Option<RelevanceClassification>,
}

/// Aggregate result of one search's experimental semantic pass.
#[derive(Debug, Clone)]
pub struct SemanticPassOutcome {
    pub evaluations: Vec<SemanticEvaluation>,
    /// Total results the search returned.
    pub results_total: usize,
    /// How many landed in the ambiguous band and were actually embedded.
    pub semantic_candidates: usize,
    /// Times the query was embedded. **Invariant: 1 when the pass runs**, 0
    /// when there were no candidates.
    pub query_embed_count: usize,
    /// Number of document embed forward passes (batched → 1 when candidates
    /// exist, 0 otherwise).
    pub document_embed_batches: usize,
    /// Set when the backend was unavailable for the whole pass; every
    /// evaluation then carries a fallback rescore.
    pub backend_fallback: Option<EmbeddingUnavailable>,
}

/// Owns a loaded [`CandleBackend`] and the query embedding for one search.
pub struct SemanticEvaluator<'a> {
    backend: &'a CandleBackend,
    config: ExperimentalSemanticConfig,
    thresholds: RelevanceThresholds,
}

impl<'a> SemanticEvaluator<'a> {
    /// Build an evaluator over an already-loaded backend.
    pub fn new(backend: &'a CandleBackend, config: ExperimentalSemanticConfig) -> Self {
        Self {
            backend,
            config,
            thresholds: RelevanceThresholds::default(),
        }
    }

    /// Convenience: validate + load a backend from a package directory, once.
    /// Returns `None` (caller falls back to bounded-semantic) on any package or
    /// load error — never panics, never hits the network.
    pub fn load_backend(
        dir: impl AsRef<std::path::Path>,
        declared_version: Option<&str>,
    ) -> Option<CandleBackend> {
        let pkg = ModelPackage::resolve(dir, declared_version).ok()?;
        CandleBackend::load(&pkg).ok()
    }

    /// Run the optimized experimental pass over a search's result set.
    ///
    /// * query embedded once;
    /// * candidate set = ambiguous-band results, capped at `k`;
    /// * documents embedded in one batch;
    /// * production assessments passed through unchanged;
    /// * suggestions computed under the fixed precedence.
    pub fn evaluate_search(
        &self,
        query: &Query,
        results: &[DeduplicatedResult],
    ) -> SemanticPassOutcome {
        let results_total = results.len();

        // Production assessment for every result (cheap, deterministic).
        let production: Vec<ResultRelevanceAssessment> = results
            .iter()
            .map(|r| assess_result(query, r, &self.thresholds))
            .collect();

        // Candidate selection: ambiguous band only, and never a result with a
        // blocking contradiction (embedding must not get a vote there).
        let mut candidates: Vec<usize> = results
            .iter()
            .zip(production.iter())
            .enumerate()
            .filter(|(_, (_, a))| self.is_ambiguous_candidate(a))
            .map(|(i, _)| i)
            .collect();
        candidates.truncate(self.config.k);

        if candidates.is_empty() {
            return SemanticPassOutcome {
                evaluations: Vec::new(),
                results_total,
                semantic_candidates: 0,
                query_embed_count: 0,
                document_embed_batches: 0,
                backend_fallback: None,
            };
        }

        let q = query.normalized_query.trim();
        if q.is_empty() {
            return self.all_fallback(
                query,
                results,
                &production,
                &candidates,
                EmbeddingUnavailable::EmptyInput,
                0,
            );
        }

        // ---- QUERY EMBEDDING: exactly once ----
        let qv = match self.backend.embed_query(q) {
            Ok(v) => v,
            Err(e) => return self.all_fallback(query, results, &production, &candidates, e, 1),
        };

        // ---- DOCUMENT EMBEDDING: one batch ----
        let texts: Vec<String> = candidates
            .iter()
            .map(|&i| document_text(&results[i]))
            .collect();
        let dvs = match self.backend.embed_documents(&texts) {
            Ok(v) => v,
            Err(e) => return self.all_fallback(query, results, &production, &candidates, e, 1),
        };

        let mut evaluations = Vec::with_capacity(candidates.len());
        for (slot, &idx) in candidates.iter().enumerate() {
            let bounded = assess_semantics(
                query,
                results[idx].title.as_deref(),
                results[idx].snippet.as_deref(),
                "",
                &Default::default(),
            );
            let cosine = cosine_similarity(&qv, &dvs[slot]);
            let rescore = if cosine.is_finite() {
                SemanticRescore {
                    cosine,
                    used_fallback: false,
                    fallback_reason: None,
                    bounded_strong_rescue: bounded.is_strong_rescue(),
                }
            } else {
                SemanticRescore {
                    cosine: 0.0,
                    used_fallback: true,
                    fallback_reason: Some(EmbeddingUnavailable::BackendError(
                        "non-finite cosine".into(),
                    )),
                    bounded_strong_rescue: bounded.is_strong_rescue(),
                }
            };
            let suggested = self.suggest(&production[idx], &rescore);
            evaluations.push(SemanticEvaluation {
                result_index: idx,
                production: production[idx].clone(),
                rescore,
                suggested,
            });
        }

        SemanticPassOutcome {
            evaluations,
            results_total,
            semantic_candidates: candidates.len(),
            query_embed_count: 1,
            document_embed_batches: 1,
            backend_fallback: None,
        }
    }

    /// A result is an ambiguous candidate iff it is `PossiblyRelevant` (or
    /// `Unknown` when configured) AND carries no blocking negative/entity
    /// evidence. The contradiction check enforces precedence rule #1:
    /// explicit negative evidence outranks any embedding signal, so those
    /// results are excluded before we spend an embed on them.
    fn is_ambiguous_candidate(&self, a: &ResultRelevanceAssessment) -> bool {
        if a.negative_evidence.entity_mismatch || a.negative_evidence.subject_mismatch {
            return false;
        }
        match a.classification {
            RelevanceClassification::PossiblyRelevant => true,
            RelevanceClassification::Unknown => self.config.include_unknown,
            _ => false,
        }
    }

    /// Suggested reclassification under the fixed precedence. Only ever
    /// `PossiblyRelevant → Relevant`, and only when:
    ///   * no blocking contradiction (already filtered, re-checked defensively);
    ///   * the embedding agrees it is a paraphrase (cosine ≥ threshold);
    ///   * the bounded-semantic layer independently corroborates
    ///     (`bounded_strong_rescue`) — lexical corroboration is required, so the
    ///     embedding is never sovereign.
    fn suggest(
        &self,
        production: &ResultRelevanceAssessment,
        rescore: &SemanticRescore,
    ) -> Option<RelevanceClassification> {
        if rescore.used_fallback {
            return None;
        }
        if production.negative_evidence.entity_mismatch
            || production.negative_evidence.subject_mismatch
        {
            return None;
        }
        if production.classification != RelevanceClassification::PossiblyRelevant {
            return None;
        }
        let embedding_agrees = rescore.cosine >= self.config.paraphrase_threshold;
        if embedding_agrees && rescore.bounded_strong_rescue {
            Some(RelevanceClassification::Relevant)
        } else {
            None
        }
    }

    fn all_fallback(
        &self,
        query: &Query,
        results: &[DeduplicatedResult],
        production: &[ResultRelevanceAssessment],
        candidates: &[usize],
        reason: EmbeddingUnavailable,
        query_embed_count: usize,
    ) -> SemanticPassOutcome {
        let evaluations = candidates
            .iter()
            .map(|&idx| {
                let bounded = assess_semantics(
                    query,
                    results[idx].title.as_deref(),
                    results[idx].snippet.as_deref(),
                    "",
                    &Default::default(),
                );
                SemanticEvaluation {
                    result_index: idx,
                    production: production[idx].clone(),
                    rescore: SemanticRescore {
                        cosine: 0.0,
                        used_fallback: true,
                        fallback_reason: Some(reason.clone()),
                        bounded_strong_rescue: bounded.is_strong_rescue(),
                    },
                    suggested: None,
                }
            })
            .collect();
        SemanticPassOutcome {
            evaluations,
            results_total: results.len(),
            semantic_candidates: candidates.len(),
            query_embed_count,
            document_embed_batches: 0,
            backend_fallback: Some(reason),
        }
    }
}

/// Canonical document representation: title then snippet, single space, trimmed.
fn document_text(r: &DeduplicatedResult) -> String {
    let mut parts: Vec<&str> = Vec::new();
    if let Some(t) = r.title.as_deref() {
        let t = t.trim();
        if !t.is_empty() {
            parts.push(t);
        }
    }
    if let Some(s) = r.snippet.as_deref() {
        let s = s.trim();
        if !s.is_empty() {
            parts.push(s);
        }
    }
    parts.join(" ")
}
