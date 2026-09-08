//! STEP 4A — local embedding relevance feasibility experiment.
//!
//! Isolated infrastructure. This crate is NOT part of the amatl workspace and
//! nothing here touches production relevance, routing, ranking or telemetry.
//!
//! The [`EmbeddingBackend`] trait is the experimental boundary. The only real
//! backend implemented in this phase is [`fastembed_backend::FastembedBackend`].

use std::collections::BTreeSet;

pub mod candle_backend;
pub mod corpus;
pub mod deterministic;
pub mod fastembed_backend;
pub mod hybrid;
pub mod lexical;

/// A fixed-size embedding vector. Convention: callers store L2-normalized
/// vectors so that [`cosine_similarity`] reduces to a dot product, but the
/// function does not assume it.
pub type Embedding = Vec<f32>;

/// Experimental embedding boundary. Deliberately minimal and decoupled from
/// providers, routing and ranking.
pub trait EmbeddingBackend {
    /// Human-readable identifier (model name + revision).
    fn id(&self) -> &str;

    /// Embedding dimensionality.
    fn dim(&self) -> usize;

    /// Embed a query string. Input is the raw user query text only — no
    /// provider identity, rank, RRF score or top-k position.
    fn embed_query(&self, text: &str) -> anyhow::Result<Embedding>;

    /// Embed a single document. Document representation is `title + snippet`
    /// only (see [`document_text`]). No provider signals.
    fn embed_document(&self, text: &str) -> anyhow::Result<Embedding>;

    /// Embed a batch of documents. Default implementation loops; backends that
    /// support true batching should override.
    fn embed_documents(&self, texts: &[String]) -> anyhow::Result<Vec<Embedding>> {
        texts.iter().map(|t| self.embed_document(t)).collect()
    }
}

/// Canonical document representation for the experiment: `title` then `snippet`,
/// each trimmed, joined by a single newline. Missing parts are skipped. This is
/// deterministic and contains no ranking-derived signal.
pub fn document_text(title: Option<&str>, snippet: Option<&str>) -> String {
    let mut parts: Vec<&str> = Vec::new();
    if let Some(t) = title {
        let t = t.trim();
        if !t.is_empty() {
            parts.push(t);
        }
    }
    if let Some(s) = snippet {
        let s = s.trim();
        if !s.is_empty() {
            parts.push(s);
        }
    }
    parts.join("\n")
}

/// Cosine similarity in `[-1.0, 1.0]`. Returns `0.0` if either vector is
/// zero-length or has zero norm (degenerate input), never NaN.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.is_empty() || b.is_empty() || a.len() != b.len() {
        return 0.0;
    }
    let mut dot = 0.0f32;
    let mut na = 0.0f32;
    let mut nb = 0.0f32;
    for i in 0..a.len() {
        dot += a[i] * b[i];
        na += a[i] * a[i];
        nb += b[i] * b[i];
    }
    if na <= f32::EPSILON || nb <= f32::EPSILON {
        return 0.0;
    }
    (dot / (na.sqrt() * nb.sqrt())).clamp(-1.0, 1.0)
}

/// The four relevance labels used across the amatl relevance corpora.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize,
)]
pub enum Label {
    #[serde(rename = "RELEVANT")]
    Relevant,
    #[serde(rename = "POSSIBLY_RELEVANT")]
    PossiblyRelevant,
    #[serde(rename = "NOT_RELEVANT")]
    NotRelevant,
    #[default]
    #[serde(rename = "UNKNOWN")]
    Unknown,
}

impl Label {
    pub fn all() -> [Label; 4] {
        [
            Label::Relevant,
            Label::PossiblyRelevant,
            Label::NotRelevant,
            Label::Unknown,
        ]
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Label::Relevant => "RELEVANT",
            Label::PossiblyRelevant => "POSSIBLY_RELEVANT",
            Label::NotRelevant => "NOT_RELEVANT",
            Label::Unknown => "UNKNOWN",
        }
    }
}

/// A 4x4 confusion matrix (rows = expected, cols = predicted) plus derived
/// metrics. Mirrors the computation in
/// `crates/amatl-core/tests/relevance_holdout_final.rs` so diagnostic numbers
/// are comparable.
#[derive(Default, Clone)]
pub struct Confusion {
    m: [[usize; 4]; 4],
}

fn idx(l: Label) -> usize {
    match l {
        Label::Relevant => 0,
        Label::PossiblyRelevant => 1,
        Label::NotRelevant => 2,
        Label::Unknown => 3,
    }
}

impl Confusion {
    pub fn record(&mut self, expected: Label, predicted: Label) {
        self.m[idx(expected)][idx(predicted)] += 1;
    }
    pub fn total(&self) -> usize {
        self.m.iter().flatten().sum()
    }
    pub fn accuracy(&self) -> f64 {
        let t = self.total();
        if t == 0 {
            return 0.0;
        }
        let correct: usize = (0..4).map(|i| self.m[i][i]).sum();
        correct as f64 / t as f64
    }
    fn prf(&self, class: Label) -> (f64, f64, f64) {
        let c = idx(class);
        let tp = self.m[c][c] as f64;
        let fp: f64 = (0..4).map(|r| self.m[r][c]).sum::<usize>() as f64 - tp;
        let fn_: f64 = self.m[c].iter().sum::<usize>() as f64 - tp;
        let p = if tp + fp > 0.0 { tp / (tp + fp) } else { 0.0 };
        let r = if tp + fn_ > 0.0 { tp / (tp + fn_) } else { 0.0 };
        let f1 = if p + r > 0.0 {
            2.0 * p * r / (p + r)
        } else {
            0.0
        };
        (p, r, f1)
    }
    pub fn precision_recall_f1(&self, class: Label) -> (f64, f64, f64) {
        self.prf(class)
    }
    pub fn macro_f1(&self) -> f64 {
        Label::all().iter().map(|l| self.prf(*l).2).sum::<f64>() / 4.0
    }
    /// NOT_RELEVANT rows predicted RELEVANT, over all NOT_RELEVANT rows.
    pub fn nr_to_r_rate(&self) -> f64 {
        let nr = idx(Label::NotRelevant);
        let total_nr: usize = self.m[nr].iter().sum();
        if total_nr == 0 {
            return 0.0;
        }
        self.m[nr][idx(Label::Relevant)] as f64 / total_nr as f64
    }
    pub fn cell(&self, expected: Label, predicted: Label) -> usize {
        self.m[idx(expected)][idx(predicted)]
    }
    pub fn render(&self) -> String {
        let mut s =
            String::from("            pred:  RELEVANT  POSSIBLY  NOT_REL   UNKNOWN | total\n");
        for e in Label::all() {
            let row = &self.m[idx(e)];
            let tot: usize = row.iter().sum();
            s.push_str(&format!(
                "  {:<16} {:8} {:9} {:8} {:9} | {:5}\n",
                e.as_str(),
                row[0],
                row[1],
                row[2],
                row[3],
                tot
            ));
        }
        s
    }
}

/// Simple whitespace/punctuation tokenizer, lowercased, for the lexical
/// baseline and for measuring token overlap in the diagnostic. Kept local so
/// this experiment does not depend on amatl-core internals.
pub fn tokens(text: &str) -> BTreeSet<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(|t| t.to_lowercase())
        .collect()
}

/// Fraction of query tokens present verbatim in the document tokens.
pub fn token_overlap(query: &str, doc: &str) -> f64 {
    let q = tokens(query);
    if q.is_empty() {
        return 0.0;
    }
    let d = tokens(doc);
    let hit = q.iter().filter(|t| d.contains(*t)).count();
    hit as f64 / q.len() as f64
}
