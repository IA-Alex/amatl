//! Loading the existing (CONSUMED) labeled relevance corpora for diagnostic
//! analysis only. These datasets must not be used as a future final gate.

use crate::Label;
use serde::Deserialize;
use std::path::Path;

#[derive(Debug, Clone, Deserialize)]
pub struct Sample {
    pub id: String,
    pub query: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub snippet: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub expected_label: String,
    #[serde(default)]
    pub query_class: Option<String>,
    #[serde(default)]
    pub provenance: Option<String>,
    #[serde(default)]
    pub confirmed_overlap: Option<bool>,
}

impl Sample {
    /// Parsed label, `None` if the field is empty or unrecognized.
    pub fn label(&self) -> Option<Label> {
        match self.expected_label.as_str() {
            "RELEVANT" => Some(Label::Relevant),
            "POSSIBLY_RELEVANT" => Some(Label::PossiblyRelevant),
            "NOT_RELEVANT" => Some(Label::NotRelevant),
            "UNKNOWN" => Some(Label::Unknown),
            _ => None,
        }
    }
    pub fn document_text(&self) -> String {
        crate::document_text(self.title.as_deref(), self.snippet.as_deref())
    }
}

#[derive(Debug, Deserialize)]
struct CorpusFile {
    #[serde(default)]
    schema: String,
    samples: Vec<Sample>,
}

pub struct Corpus {
    pub schema: String,
    pub samples: Vec<Sample>,
}

pub fn load(path: impl AsRef<Path>) -> anyhow::Result<Corpus> {
    let bytes = std::fs::read(path.as_ref())?;
    let f: CorpusFile = serde_json::from_slice(&bytes)?;
    Ok(Corpus {
        schema: f.schema,
        samples: f.samples,
    })
}

/// Locate the fixtures dir relative to this crate (../../crates/amatl-core/...).
pub fn fixtures_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../crates/amatl-core/tests/fixtures/relevance")
        .canonicalize()
        .unwrap_or_else(|_| {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../crates/amatl-core/tests/fixtures/relevance")
        })
}

/// Coarse diagnostic bucket derived from `query_class` and label. Used to
/// report per-failure-mode deltas rather than only aggregate accuracy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Bucket {
    /// RELEVANT but low lexical overlap — the paraphrase / synonymy class.
    ParaphraseRelevant,
    /// NOT_RELEVANT with non-trivial lexical overlap — lexical collision /
    /// wrong-sense.
    LexicalCollisionNegative,
    /// Entity/topic mismatch negatives.
    EntityMismatch,
    /// POSSIBLY_RELEVANT / partial.
    Partial,
    /// UNKNOWN / insufficient evidence.
    InsufficientEvidence,
    /// Everything else (high-overlap relevant, generic negatives).
    Other,
}

impl Bucket {
    pub fn classify(s: &Sample) -> Bucket {
        let overlap = crate::token_overlap(&s.query, &s.document_text());
        let class = s.query_class.as_deref().unwrap_or("");
        match s.label() {
            Some(Label::Relevant) if overlap < 0.5 => Bucket::ParaphraseRelevant,
            Some(Label::NotRelevant) if overlap >= 0.34 => Bucket::LexicalCollisionNegative,
            Some(Label::NotRelevant) => Bucket::EntityMismatch,
            Some(Label::PossiblyRelevant) => Bucket::Partial,
            Some(Label::Unknown) => Bucket::InsufficientEvidence,
            _ if class == "INSUFFICIENT_EVIDENCE" => Bucket::InsufficientEvidence,
            _ => Bucket::Other,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Bucket::ParaphraseRelevant => "PARAPHRASE_RELEVANT",
            Bucket::LexicalCollisionNegative => "LEXICAL_COLLISION_NEGATIVE",
            Bucket::EntityMismatch => "ENTITY_MISMATCH",
            Bucket::Partial => "PARTIAL",
            Bucket::InsufficientEvidence => "INSUFFICIENT_EVIDENCE",
            Bucket::Other => "OTHER",
        }
    }
}
