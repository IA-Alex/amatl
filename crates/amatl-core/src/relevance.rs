//! STEP 2E — DETERMINISTIC RELEVANCE SIGNAL (first version).
//!
//! Evolves the result contract from `VALID_RESULT` → `RELEVANT_RESULT` →
//! `UNIQUE_RELEVANT_RESULT`. The complementarity contract
//! ([`crate::compute_complementarity_metrics`]) tells us what is *structurally*
//! unique; this layer tells us what is *probably relevant to the query*.
//!
//! ## Design constraints (all satisfied here)
//!
//! * deterministic — same inputs, byte-identical output;
//! * explainable — every component of the decision is kept in
//!   [`ResultRelevanceAssessment`], never collapsed to one opaque float;
//! * local — no network, no LLM, no embeddings, no remote inference;
//! * cheap — set intersection over already-normalized tokens;
//! * non-circular — computed from title/snippet/url text + provider-original
//!   rank only. It never reads the final ranking position, provider agreement,
//!   or dedupe-derived scores, so there is no
//!   RELEVANCE → RANKING → RELEVANCE loop. It can be evaluated before ranking.
//!
//! ## Circularity note
//!
//! `provider_rank_is_top` is the provider's *own* rank (`provider_ranks`), which
//! exists before AMATL ranks anything. It is a **secondary** signal: it can
//! nudge a `PossiblyRelevant` result but can never, alone, make a result
//! `Relevant`. AMATL's final `rank`, `combined_score`, `provider_agreement` and
//! RRF are never consulted.
//!
//! ## Tokenization
//!
//! Reuses [`crate::text`] (`normalized_text` / `tokens`) — the exact same
//! normalization `ranking` and `classify` use. No second, incompatible
//! tokenizer. Significant query terms = query tokens with the extremely common
//! function words removed via a small, fixed stop list; quoted-phrase terms are
//! always kept significant.

use crate::model::{
    ComplementarityMetrics, DeduplicatedResult, Query, RelevanceAssessmentStatus,
    RelevanceClassification, RelevanceMetrics, ResultRelevanceAssessment,
};
use crate::router::{ProviderRole, RoleAssignment};
use crate::text::{normalized_text, tokens};
use std::collections::BTreeSet;

/// Deterministic thresholds for the first heuristic.
///
/// **INITIAL_HEURISTIC** — these numbers are conservative starting points, not
/// empirically optimized values. They are deliberately *not* called "optimal".
/// They must be validated against real query/result corpora before any
/// telemetry or adaptive routing is built on this signal.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RelevanceThresholds {
    /// Title coverage at or above this alone makes a result `Relevant`.
    pub relevant_title_coverage: f64,
    /// Combined evidence: `title_coverage + 0.5 * snippet_coverage` at or above
    /// this makes a result `Relevant`.
    pub relevant_combined_coverage: f64,
    /// Snippet coverage at or above this, with at least two distinct matched
    /// terms, alone makes a result `Relevant` (term distribution guard stops a
    /// single trivial hit from qualifying).
    pub relevant_snippet_coverage: f64,
    /// Any single field's coverage at or above this is "non-trivial" partial
    /// evidence → at least `PossiblyRelevant`.
    pub possibly_relevant_coverage: f64,
    /// A result with sufficient text but combined coverage strictly below this
    /// (and no phrase / url hint) is `NotRelevant`.
    pub not_relevant_ceiling: f64,
    /// A title shorter than this many significant tokens, with no snippet, is
    /// treated as "generic / insufficient text" → `Unknown`.
    pub min_title_tokens_without_snippet: usize,
}

impl Default for RelevanceThresholds {
    fn default() -> Self {
        Self {
            relevant_title_coverage: 0.75,
            relevant_combined_coverage: 1.0,
            relevant_snippet_coverage: 0.90,
            possibly_relevant_coverage: 0.40,
            not_relevant_ceiling: 0.25,
            min_title_tokens_without_snippet: 2,
        }
    }
}

/// Extremely common words that must not, on their own, satisfy relevance.
/// Small and fixed — not a linguistic stemmer. Kept deliberately short so it is
/// auditable and stable.
const STOP_WORDS: &[&str] = &[
    "a", "an", "and", "are", "as", "at", "be", "by", "for", "from", "how", "in", "is", "it", "of",
    "on", "or", "the", "to", "vs", "what", "when", "where", "which", "who", "why", "with",
];

/// Significant query terms: normalized query tokens minus stop words, plus every
/// token from a quoted phrase (those are always significant). Order-independent.
fn significant_query_terms(query: &Query) -> BTreeSet<String> {
    let mut terms: BTreeSet<String> = tokens(&query.normalized_query)
        .into_iter()
        .filter(|t| !STOP_WORDS.contains(&t.as_str()))
        .collect();
    for phrase in &query.quoted_terms {
        terms.extend(tokens(phrase));
    }
    terms
}

/// Normalized quoted phrases, non-empty only.
fn quoted_phrases(query: &Query) -> Vec<String> {
    query
        .quoted_terms
        .iter()
        .map(|p| normalized_text(p))
        .filter(|p| !p.is_empty())
        .collect()
}

fn coverage(field: Option<&str>, terms: &BTreeSet<String>) -> (f64, BTreeSet<String>) {
    let Some(field) = field else {
        return (0.0, BTreeSet::new());
    };
    if terms.is_empty() {
        return (0.0, BTreeSet::new());
    }
    let field_tokens = tokens(field);
    let matched: BTreeSet<String> = terms.intersection(&field_tokens).cloned().collect();
    (matched.len() as f64 / terms.len() as f64, matched)
}

fn url_tokens(result: &DeduplicatedResult) -> String {
    let url = &result.canonical_url.0;
    let mut parts = Vec::new();
    if let Some(host) = url.host_str() {
        parts.push(host.to_string());
    }
    parts.push(url.path().replace(['/', '-', '_', '.'], " "));
    parts.join(" ")
}

/// Assess one result against the query with the given thresholds.
///
/// Pure: depends only on `query`, the result's own text/url/provider-rank, and
/// `thresholds`.
pub fn assess_result(
    query: &Query,
    result: &DeduplicatedResult,
    thresholds: &RelevanceThresholds,
) -> ResultRelevanceAssessment {
    let terms = significant_query_terms(query);
    let phrases = quoted_phrases(query);

    let title_text = result.title.as_deref();
    let snippet_text = result.snippet.as_deref();

    let (title_cov, title_matched) = coverage(title_text, &terms);
    let (snippet_cov, snippet_matched) = coverage(snippet_text, &terms);
    let url_blob = url_tokens(result);
    let (url_cov, url_matched) = coverage(Some(url_blob.as_str()), &terms);

    let mut all_matched = title_matched;
    all_matched.extend(snippet_matched);
    all_matched.extend(url_matched);

    // Exact quoted-phrase match: normalized phrase appears literally in the
    // normalized title or snippet.
    let normalized_title = title_text.map(normalized_text).unwrap_or_default();
    let normalized_snippet = snippet_text.map(normalized_text).unwrap_or_default();
    let exact_phrase_match = !phrases.is_empty()
        && phrases.iter().any(|p| {
            normalized_title.contains(p.as_str()) || normalized_snippet.contains(p.as_str())
        });

    let title_tokens = title_text.map(|t| tokens(t).len()).unwrap_or(0);
    let has_snippet = snippet_text.map(|s| !s.trim().is_empty()).unwrap_or(false);
    // "Sufficient text": a snippet, or a title with at least a couple of
    // significant-length tokens. A one-word or empty title with no snippet is
    // not enough to judge.
    let had_sufficient_text =
        has_snippet || title_tokens >= thresholds.min_title_tokens_without_snippet.max(1);

    let provider_rank_is_top = result
        .provider_ranks
        .values()
        .filter_map(|r| *r)
        .map(|r| r.get() == 1)
        .fold(None, |acc, is_top| Some(acc.unwrap_or(false) || is_top));

    let combined = title_cov + 0.5 * snippet_cov;

    let strong_snippet =
        snippet_cov >= thresholds.relevant_snippet_coverage && all_matched.len() >= 2;

    let classification = if !had_sufficient_text {
        RelevanceClassification::Unknown
    } else if exact_phrase_match
        || title_cov >= thresholds.relevant_title_coverage
        || combined >= thresholds.relevant_combined_coverage
        || strong_snippet
    {
        RelevanceClassification::Relevant
    } else if title_cov >= thresholds.possibly_relevant_coverage
        || snippet_cov >= thresholds.possibly_relevant_coverage
        || combined >= thresholds.possibly_relevant_coverage
        || (url_cov >= thresholds.possibly_relevant_coverage && !all_matched.is_empty())
        // secondary nudge: some textual overlap AND provider ranked it first.
        || (provider_rank_is_top == Some(true) && !all_matched.is_empty() && combined > 0.0)
    {
        RelevanceClassification::PossiblyRelevant
    } else if has_snippet && combined <= thresholds.not_relevant_ceiling {
        // NotRelevant requires a snippet: affirming "no relation" needs real
        // body text. A missing snippet with a weak title stays Unknown.
        RelevanceClassification::NotRelevant
    } else if has_snippet {
        RelevanceClassification::PossiblyRelevant
    } else {
        RelevanceClassification::Unknown
    };

    ResultRelevanceAssessment {
        title_term_coverage: title_cov,
        snippet_term_coverage: snippet_cov,
        url_term_coverage: url_cov,
        exact_phrase_match,
        matched_query_terms: all_matched.len() as u32,
        query_terms_total: terms.len() as u32,
        had_sufficient_text,
        provider_rank_is_top,
        classification,
    }
}

/// Aggregate the deterministic relevance signal over a post-dedupe result set.
///
/// Consumes the *already computed* [`ComplementarityMetrics`] for the raw
/// structural counts and role/overlap identity — it does not re-derive roles,
/// re-compare URLs, or re-run overlap detection. Returns `None` in legacy mode
/// (mirrors `compute_complementarity_metrics`).
///
/// This is an **observation**. Nothing here feeds `AdaptiveRouter`.
pub fn assess_relevance(
    query: &Query,
    deduped: &[DeduplicatedResult],
    roles: &RoleAssignment,
    complementarity: &ComplementarityMetrics,
    thresholds: &RelevanceThresholds,
) -> Option<RelevanceMetrics> {
    if !roles.active {
        return None;
    }

    let has_role = |result: &DeduplicatedResult, want: ProviderRole| {
        result
            .providers
            .iter()
            .any(|name| roles.role_of(name) == Some(want))
    };

    let mut relevant_primary = 0u32;
    let mut relevant_expansion = 0u32;
    let mut unique_relevant_primary = 0u32;
    let mut unique_relevant_expansion = 0u32;
    let mut possibly_primary = 0u32;
    let mut possibly_expansion = 0u32;
    let mut not_primary = 0u32;
    let mut not_expansion = 0u32;
    let mut unknown_primary = 0u32;
    let mut unknown_expansion = 0u32;

    // EXPANSION-exclusive conclusive tallies for the confirmed noise rate.
    let mut expansion_excl_relevant = 0u32;
    let mut expansion_excl_not_relevant = 0u32;

    let mut assessable = 0u32;

    for result in deduped {
        let in_primary = has_role(result, ProviderRole::Primary);
        let in_expansion = has_role(result, ProviderRole::Expansion);
        if !in_primary && !in_expansion {
            continue;
        }
        let confirmed_overlap = in_primary && in_expansion;
        let assessment = assess_result(query, result, thresholds);
        if assessment.had_sufficient_text {
            assessable += 1;
        }

        use RelevanceClassification::*;
        if in_primary {
            match assessment.classification {
                Relevant => {
                    relevant_primary += 1;
                    if !confirmed_overlap {
                        unique_relevant_primary += 1;
                    }
                }
                PossiblyRelevant => possibly_primary += 1,
                NotRelevant => not_primary += 1,
                Unknown => unknown_primary += 1,
            }
        }
        if in_expansion {
            match assessment.classification {
                Relevant => {
                    relevant_expansion += 1;
                    if !confirmed_overlap {
                        unique_relevant_expansion += 1;
                        expansion_excl_relevant += 1;
                    }
                }
                PossiblyRelevant => possibly_expansion += 1,
                NotRelevant => {
                    not_expansion += 1;
                    if !confirmed_overlap {
                        expansion_excl_not_relevant += 1;
                    }
                }
                Unknown => unknown_expansion += 1,
            }
        }
    }

    let found_expansion = complementarity.found_expansion;
    let unique_expansion_raw = complementarity.unique_expansion;

    let expansion_relevant_ratio = Some(relevant_expansion as f64 / found_expansion.max(1) as f64);
    let raw_to_relevant_expansion_ratio =
        Some(unique_relevant_expansion as f64 / unique_expansion_raw.max(1) as f64);

    // Confirmed noise rate: denominator is conclusive EXPANSION-exclusive
    // assessments only (Relevant + NotRelevant). Possible / Unknown never count.
    let conclusive = expansion_excl_relevant + expansion_excl_not_relevant;
    let expansion_confirmed_noise_rate =
        Some(expansion_excl_not_relevant as f64 / conclusive.max(1) as f64);

    let total_role_results = deduped
        .iter()
        .filter(|r| has_role(r, ProviderRole::Primary) || has_role(r, ProviderRole::Expansion))
        .count() as u32;
    let status = if total_role_results > 0 && assessable * 2 < total_role_results {
        RelevanceAssessmentStatus::InsufficientEvidence
    } else {
        RelevanceAssessmentStatus::Assessed
    };

    Some(RelevanceMetrics {
        status,
        relevant_primary: Some(relevant_primary),
        relevant_expansion: Some(relevant_expansion),
        unique_relevant_primary: Some(unique_relevant_primary),
        unique_relevant_expansion: Some(unique_relevant_expansion),
        possibly_relevant_primary: Some(possibly_primary),
        possibly_relevant_expansion: Some(possibly_expansion),
        not_relevant_primary: Some(not_primary),
        not_relevant_expansion: Some(not_expansion),
        unknown_primary: Some(unknown_primary),
        unknown_expansion: Some(unknown_expansion),
        expansion_relevant_ratio,
        raw_to_relevant_expansion_ratio,
        expansion_confirmed_noise_rate,
        expansion_noise_rate: expansion_confirmed_noise_rate,
    })
}

/// Convenience: compute complementarity then enrich it with the relevance
/// layer, leaving the structural fields untouched. `unique_expansion` (raw) is
/// preserved exactly as `compute_complementarity_metrics` produced it.
pub fn enrich_relevance_metrics(
    query: &Query,
    deduped: &[DeduplicatedResult],
    roles: &RoleAssignment,
    thresholds: &RelevanceThresholds,
    metrics: &mut ComplementarityMetrics,
) {
    if let Some(relevance) = assess_relevance(query, deduped, roles, metrics, thresholds) {
        metrics.relevance = relevance;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{
        CanonicalUrl, DuplicateStatus, OriginalUrl, RelevanceClassification, ResultType,
        SCHEMA_VERSION,
    };
    use crate::parse_query;
    use std::collections::BTreeMap;

    fn result(
        url: &str,
        title: Option<&str>,
        snippet: Option<&str>,
        providers: &[&str],
        rank: Option<u32>,
    ) -> DeduplicatedResult {
        let parsed = url::Url::parse(url).unwrap();
        DeduplicatedResult {
            schema_version: SCHEMA_VERSION.into(),
            title: title.map(str::to_string),
            original_url: OriginalUrl(parsed.clone()),
            canonical_url: CanonicalUrl(parsed.clone()),
            original_urls: vec![OriginalUrl(parsed)],
            providers: providers.iter().map(|p| p.to_string()).collect(),
            representative_provider: providers.first().copied().unwrap_or("").into(),
            provider_ranks: providers
                .iter()
                .map(|p| {
                    (
                        (*p).to_string(),
                        rank.and_then(|v| crate::Rank::new(v).ok()),
                    )
                })
                .collect::<BTreeMap<_, _>>(),
            snippet: snippet.map(str::to_string),
            alternate_snippets: vec![],
            result_type: ResultType::Organic,
            published_at: None,
            author: None,
            language: None,
            file_type: None,
            thumbnail: None,
            metadata: BTreeMap::new(),
            observed_dates: vec![],
            duplicate_status: if providers.len() > 1 {
                DuplicateStatus::ConfirmedDuplicate
            } else {
                DuplicateStatus::Distinct
            },
            merge_reason: None,
            possible_duplicate_with: vec![],
        }
    }

    fn classify(q: &str, title: Option<&str>, snippet: Option<&str>) -> RelevanceClassification {
        let query = parse_query(q.to_string()).unwrap();
        let r = result("https://example.com/page", title, snippet, &["p"], None);
        assess_result(&query, &r, &RelevanceThresholds::default()).classification
    }

    #[test]
    fn strong_title_match_is_relevant() {
        assert_eq!(
            classify(
                "rust async programming",
                Some("Async programming in Rust"),
                Some("Learn asynchronous programming using async and await in Rust."),
            ),
            RelevanceClassification::Relevant
        );
    }

    #[test]
    fn strong_snippet_match_is_relevant() {
        // Weak title, but snippet carries every term -> combined >= 1.0.
        assert_eq!(
            classify(
                "rust async programming",
                Some("A blog post"),
                Some("rust async programming: a full guide to async programming in rust"),
            ),
            RelevanceClassification::Relevant
        );
    }

    #[test]
    fn exact_quoted_phrase_is_relevant() {
        assert_eq!(
            classify(
                "\"capital of Canada\"",
                Some("Cities and towns"),
                Some("The capital of Canada is Ottawa."),
            ),
            RelevanceClassification::Relevant
        );
    }

    #[test]
    fn unrelated_result_is_not_relevant() {
        assert_eq!(
            classify(
                "capital of Canada",
                Some("Political opinions and independent commentary"),
                Some("A collection of personal essays about music and gardening."),
            ),
            RelevanceClassification::NotRelevant
        );
    }

    #[test]
    fn partial_match_is_possibly_relevant() {
        assert_eq!(
            classify(
                "rust async programming tokio",
                Some("Rust programming basics"),
                Some("An introduction to programming."),
            ),
            RelevanceClassification::PossiblyRelevant
        );
    }

    #[test]
    fn insufficient_text_is_unknown() {
        assert_eq!(
            classify("capital of Canada", Some("Home"), None),
            RelevanceClassification::Unknown
        );
    }

    #[test]
    fn missing_snippet_does_not_force_not_relevant() {
        // Strong title, no snippet -> still Relevant.
        assert_eq!(
            classify(
                "rust async programming",
                Some("Rust async programming guide"),
                None
            ),
            RelevanceClassification::Relevant
        );
        // Generic title, no snippet -> Unknown, NOT NotRelevant.
        assert_eq!(
            classify("rust async programming", Some("Documentation index"), None),
            RelevanceClassification::Unknown
        );
    }

    #[test]
    fn identical_inputs_produce_identical_relevance() {
        let query = parse_query("rust async programming".into()).unwrap();
        let r = result(
            "https://example.com/x",
            Some("Async programming in Rust"),
            Some("async await in rust"),
            &["p"],
            Some(1),
        );
        let a = assess_result(&query, &r, &RelevanceThresholds::default());
        let b = assess_result(&query, &r, &RelevanceThresholds::default());
        assert_eq!(a, b);
    }

    fn roles() -> RoleAssignment {
        RoleAssignment::primary_expansion("marginalia", ["searxng".to_string()])
    }

    #[test]
    fn unique_irrelevant_expansion_is_not_unique_relevant() {
        let query = parse_query("capital of Canada".into()).unwrap();
        let set = [
            result(
                "https://a.example/1",
                Some("The capital of Canada"),
                Some("Ottawa is the capital of Canada."),
                &["marginalia"],
                Some(1),
            ),
            // EXPANSION-exclusive but pure noise.
            result(
                "https://b.example/2",
                Some("Independent political commentary and opinions"),
                Some("Essays on music, gardening and travel."),
                &["searxng"],
                Some(1),
            ),
        ];
        let comp = crate::compute_complementarity_metrics(&set, &roles()).unwrap();
        assert_eq!(comp.unique_expansion, 1, "raw structural count preserved");
        let rel = assess_relevance(
            &query,
            &set,
            &roles(),
            &comp,
            &RelevanceThresholds::default(),
        )
        .unwrap();
        assert_eq!(rel.unique_relevant_expansion, Some(0));
        assert_eq!(rel.not_relevant_expansion, Some(1));
    }

    #[test]
    fn relevant_expansion_increments_unique_relevant_expansion() {
        let query = parse_query("rust async programming".into()).unwrap();
        let set = [result(
            "https://b.example/2",
            Some("Async programming in Rust"),
            Some("async and await in rust"),
            &["searxng"],
            Some(1),
        )];
        let comp = crate::compute_complementarity_metrics(&set, &roles()).unwrap();
        let rel = assess_relevance(
            &query,
            &set,
            &roles(),
            &comp,
            &RelevanceThresholds::default(),
        )
        .unwrap();
        assert_eq!(rel.unique_relevant_expansion, Some(1));
    }

    #[test]
    fn confirmed_overlap_is_not_unique_relevant_expansion() {
        let query = parse_query("rust async programming".into()).unwrap();
        let set = [result(
            "https://b.example/2",
            Some("Async programming in Rust"),
            Some("async and await in rust"),
            &["marginalia", "searxng"],
            Some(1),
        )];
        let comp = crate::compute_complementarity_metrics(&set, &roles()).unwrap();
        let rel = assess_relevance(
            &query,
            &set,
            &roles(),
            &comp,
            &RelevanceThresholds::default(),
        )
        .unwrap();
        assert_eq!(rel.relevant_expansion, Some(1));
        assert_eq!(rel.unique_relevant_expansion, Some(0));
        assert_eq!(rel.relevant_primary, Some(1));
        assert_eq!(rel.unique_relevant_primary, Some(0));
    }

    #[test]
    fn unknown_results_do_not_count_as_confirmed_noise() {
        let query = parse_query("rust async programming".into()).unwrap();
        let set = [
            result(
                "https://b.example/2",
                Some("Index"),
                None,
                &["searxng"],
                Some(1),
            ),
            result(
                "https://c.example/3",
                Some("Async programming in Rust"),
                Some("async await rust"),
                &["searxng"],
                Some(1),
            ),
        ];
        let comp = crate::compute_complementarity_metrics(&set, &roles()).unwrap();
        let rel = assess_relevance(
            &query,
            &set,
            &roles(),
            &comp,
            &RelevanceThresholds::default(),
        )
        .unwrap();
        assert_eq!(rel.unknown_expansion, Some(1));
        // one relevant, zero not-relevant, unknown excluded -> 0.0
        assert_eq!(rel.expansion_confirmed_noise_rate, Some(0.0));
    }

    #[test]
    fn noise_rate_has_safe_denominator() {
        let query = parse_query("rust async programming".into()).unwrap();
        // No conclusive EXPANSION-exclusive results at all.
        let set = [result(
            "https://b.example/2",
            Some("Index"),
            None,
            &["searxng"],
            Some(1),
        )];
        let comp = crate::compute_complementarity_metrics(&set, &roles()).unwrap();
        let rel = assess_relevance(
            &query,
            &set,
            &roles(),
            &comp,
            &RelevanceThresholds::default(),
        )
        .unwrap();
        let rate = rel.expansion_confirmed_noise_rate.unwrap();
        assert!(rate.is_finite() && (0.0..=1.0).contains(&rate));
    }
}
