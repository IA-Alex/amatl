//! Mode B proxy — "bounded semantic". A small deterministic classifier that
//! adds morphological (stemming-lite) and negative-evidence signals on top of
//! the lexical baseline.
//!
//! NOTE: this is a local *proxy* for amatl-core's production bounded-semantic
//! assessment, not a call into it — the experiment crate is isolated from the
//! workspace. It is calibrated only on the CONSUMED corpora and is used to give
//! the A/B/C/D diagnostic a "no-embedding semantic" reference column.

use crate::{tokens, Label};
use std::collections::BTreeSet;

/// Very small suffix stripper — enough to fold common inflections.
fn stem(t: &str) -> String {
    for suf in ["ingly", "edly", "ing", "ers", "er", "ed", "es", "s", "ly"] {
        if t.len() > suf.len() + 2 && t.ends_with(suf) {
            return t[..t.len() - suf.len()].to_string();
        }
    }
    t.to_string()
}

fn stem_set(s: &BTreeSet<String>) -> BTreeSet<String> {
    s.iter().map(|t| stem(t)).collect()
}

/// Hand-built concept alias groups (small, mirrors the spirit of amatl-core's
/// ConceptAliasSet). Diagnostic only.
const ALIAS_GROUPS: &[&[&str]] = &[
    &["burnout", "exhaustion", "fatigue", "overwork"],
    &["faster", "speed", "performance", "optimization", "latency"],
    &["car", "vehicle", "automobile", "engine"],
    &["docs", "documentation", "reference", "manual", "guide"],
    &["auth", "authentication", "login", "signin", "oauth"],
    &["db", "database", "sql", "postgres", "sqlite", "mysql"],
    &["k8s", "kubernetes", "container", "pod"],
    &["ml", "machine-learning", "model", "training", "inference"],
];

fn concepts(toks: &BTreeSet<String>) -> BTreeSet<usize> {
    let mut out = BTreeSet::new();
    for (i, group) in ALIAS_GROUPS.iter().enumerate() {
        if group.iter().any(|g| toks.contains(*g)) {
            out.insert(i);
        }
    }
    out
}

/// Negative evidence: the document looks like it is about a *different* named
/// entity than the query (very rough — capitalized-ish token in doc not in
/// query and vice versa is not modeled; we approximate with "query has a
/// distinctive rare token entirely absent from a non-empty document").
fn blocking_negative(q: &BTreeSet<String>, d: &BTreeSet<String>) -> bool {
    if d.is_empty() {
        return false;
    }
    let distinctive: Vec<&String> = q.iter().filter(|t| t.len() >= 5).collect();
    if distinctive.is_empty() {
        return false;
    }
    let ds = stem_set(d);
    distinctive
        .iter()
        .all(|t| !d.contains(*t) && !ds.contains(&stem(t)))
}

#[derive(Debug, Clone, Default)]
pub struct Assessment {
    pub lexical_overlap: f64,
    pub stemmed_matches: usize,
    pub alias_matches: usize,
    pub negative_block: bool,
    pub label: Label,
}

pub fn assess(query: &str, doc: &str) -> Assessment {
    let q = tokens(query);
    let d = tokens(doc);
    if doc.trim().is_empty() {
        return Assessment {
            label: Label::Unknown,
            ..Default::default()
        };
    }
    let qs = stem_set(&q);
    let ds = stem_set(&d);
    let exact = q.iter().filter(|t| d.contains(*t)).count();
    let lexical_overlap = if q.is_empty() {
        0.0
    } else {
        exact as f64 / q.len() as f64
    };
    let stemmed_matches = qs
        .iter()
        .filter(|s| ds.contains(*s))
        .count()
        .saturating_sub(exact);
    let alias_matches = concepts(&q).intersection(&concepts(&d)).count();
    let negative_block = blocking_negative(&q, &d);

    // Decision: lexical, lifted by morphology/alias evidence, capped by
    // negative evidence.
    let effective = lexical_overlap + 0.12 * stemmed_matches as f64 + 0.20 * alias_matches as f64;

    let label = if negative_block {
        Label::NotRelevant
    } else if effective >= 0.75 || (effective >= 0.5 && alias_matches >= 1) {
        Label::Relevant
    } else if effective >= 0.34 {
        Label::PossiblyRelevant
    } else {
        Label::NotRelevant
    };

    Assessment {
        lexical_overlap,
        stemmed_matches,
        alias_matches,
        negative_block,
        label,
    }
}
