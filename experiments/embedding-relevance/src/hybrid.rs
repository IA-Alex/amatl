//! Mode C (embedding-only) and Mode D (bounded-semantic + embedding
//! corroboration).
//!
//! Design constraint (spec §6): embedding evidence must not blindly override
//! explicit high-confidence contradiction, and every decision keeps auditable
//! fields. No opaque single weighted score.

use crate::deterministic::{self, Assessment as DetAssessment};
use crate::Label;

/// Similarity bands for bge-small-en-v1.5 cosine on `query` vs `title+snippet`.
/// Calibrated ONLY on the consumed diagnostic corpora — not a generalization
/// claim.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SemBand {
    /// Clearly on-topic.
    High,
    /// Plausibly related.
    Medium,
    /// Weak / off-topic.
    Low,
}

pub fn band(sim: f32, t_high: f32, t_med: f32) -> SemBand {
    if sim >= t_high {
        SemBand::High
    } else if sim >= t_med {
        SemBand::Medium
    } else {
        SemBand::Low
    }
}

/// Mode C — embedding similarity only.
pub fn embedding_only(sim: f32, doc_empty: bool, t_high: f32, t_med: f32) -> Label {
    if doc_empty {
        return Label::Unknown;
    }
    match band(sim, t_high, t_med) {
        SemBand::High => Label::Relevant,
        SemBand::Medium => Label::PossiblyRelevant,
        SemBand::Low => Label::NotRelevant,
    }
}

/// Auditable record of a Mode D decision.
#[derive(Debug, Clone)]
pub struct HybridDecision {
    pub lexical_assessment: Label,
    pub bounded_semantic: Label,
    pub semantic_similarity: f32,
    pub semantic_band: SemBand,
    pub negative_evidence: bool,
    pub experimental_final_assessment: Label,
    pub reason: &'static str,
}

/// Mode D — bounded semantic, corroborated (or moderated) by embedding evidence.
///
/// Rules, in order:
///  1. If bounded-semantic found *blocking negative evidence*, that stands.
///     Embedding High can downgrade NOT_RELEVANT to POSSIBLY_RELEVANT at most,
///     never to RELEVANT (no blind override of contradiction).
///  2. Otherwise, embedding High corroborates / rescues:
///     - bounded says RELEVANT  -> RELEVANT
///     - bounded says POSSIBLY   -> RELEVANT   (semantic rescue)
///     - bounded says NOT_RELEVANT (no neg evidence) -> POSSIBLY_RELEVANT
///  3. Embedding Medium: keep bounded, but lift NOT_RELEVANT (no neg) ->
///     POSSIBLY_RELEVANT.
///  4. Embedding Low: keep bounded, but cap a *lexical-only* RELEVANT (no
///     alias/stemmed support) down to POSSIBLY_RELEVANT — guards lexical
///     collisions.
///  5. Empty document / empty query -> UNKNOWN preserved.
pub fn hybrid(query: &str, doc: &str, sim: f32, t_high: f32, t_med: f32) -> HybridDecision {
    let det: DetAssessment = deterministic::assess(query, doc);
    let lexical = crate::lexical::classify(query, doc);
    let b = band(sim, t_high, t_med);
    let doc_empty = doc.trim().is_empty();
    let q_empty = query.trim().is_empty();

    if doc_empty || q_empty {
        return HybridDecision {
            lexical_assessment: lexical,
            bounded_semantic: det.label,
            semantic_similarity: sim,
            semantic_band: b,
            negative_evidence: det.negative_block,
            experimental_final_assessment: Label::Unknown,
            reason: "insufficient evidence (empty query or document)",
        };
    }

    let lexical_only_relevant =
        det.label == Label::Relevant && det.alias_matches == 0 && det.stemmed_matches == 0;

    let (final_label, reason): (Label, &'static str) = if det.negative_block {
        match b {
            SemBand::High => (
                Label::PossiblyRelevant,
                "negative evidence, softened by high semantic sim",
            ),
            _ => (Label::NotRelevant, "blocking negative evidence stands"),
        }
    } else {
        match b {
            SemBand::High => match det.label {
                Label::Relevant => (Label::Relevant, "bounded + embedding agree relevant"),
                Label::PossiblyRelevant => (Label::Relevant, "embedding rescues paraphrase"),
                Label::NotRelevant => (Label::PossiblyRelevant, "embedding lifts weak lexical"),
                Label::Unknown => (Label::PossiblyRelevant, "embedding suggests topical"),
            },
            SemBand::Medium => match det.label {
                Label::NotRelevant => (Label::PossiblyRelevant, "embedding medium lifts NR"),
                other => (other, "bounded kept, embedding medium"),
            },
            SemBand::Low => {
                if lexical_only_relevant {
                    (
                        Label::PossiblyRelevant,
                        "low sim caps lexical-only relevant",
                    )
                } else {
                    (det.label, "bounded kept, embedding low")
                }
            }
        }
    };

    HybridDecision {
        lexical_assessment: lexical,
        bounded_semantic: det.label,
        semantic_similarity: sim,
        semantic_band: b,
        negative_evidence: det.negative_block,
        experimental_final_assessment: final_label,
        reason,
    }
}
