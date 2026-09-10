//! Post-rank ranking adjustments that live *outside* the protected
//! `RankingPolicyV1` weighted-sum formula.
//!
//! `ranking::rank()` computes a score from the five contract weights that
//! `RankingPolicyV1::validate()` requires to sum to exactly 1.0
//! (fase_a_contratos.md). Anything that wants to nudge a result up or down
//! without being one of those weights belongs here instead: this module runs
//! once, after `rank()`, and scales scores multiplicatively so the protected
//! formula and its "weights sum to 1.0" invariant are never touched.
//!
//! Today the only adjustment is a tracker/ad-domain penalty. Future pieces
//! (a curated-independent-web bonus, user-defined Optics rules) each get their
//! own function here and are folded into [`apply_adjustments`] in sequence;
//! `execution.rs` calls this one entry point and never needs to change again.

use crate::model::{CanonicalUrl, RankedResult, RankingScore};
use std::collections::HashSet;
use std::sync::LazyLock;

/// Domains from stevenblack/hosts (MIT), bundled at compile time.
/// Source: github.com/StevenBlack/hosts. See the file header for the
/// snapshot date -- re-run the PASO 2 transform periodically to refresh.
static TRACKER_DOMAINS: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    include_str!("../assets/tracker-domains.txt")
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .collect()
});

/// How much a tracker-domain match multiplies the final score by.
/// Deliberately a multiplier, not a subtraction: it scales down a result
/// regardless of how well it otherwise scored, without needing to touch
/// RankingPolicyV1's protected weighted-sum formula (fase_a_contratos.md)
/// or its "weights must sum to 1.0" invariant.
const TRACKER_PENALTY_MULTIPLIER: f64 = 0.05;

/// Applies post-rank adjustments that live outside the protected
/// RankingPolicyV1 formula. Called once, after `ranking::rank()`, before
/// the results are returned to the caller. Future adjustments (curated-
/// list bonus, Optics rules) get their own function here and get folded
/// into `apply_adjustments` in sequence -- this function is the single
/// entry point execution.rs calls, so adding a new adjustment never
/// means touching execution.rs again.
pub fn apply_adjustments(mut ranked: Vec<RankedResult>) -> Vec<RankedResult> {
    for result in &mut ranked {
        if is_tracker_domain(&result.result.canonical_url) {
            let adjusted = (result.score.get() * TRACKER_PENALTY_MULTIPLIER).clamp(0.0, 1.0);
            result.score =
                RankingScore::new(adjusted).expect("clamped value is always within 0.0..=1.0");
        }
    }
    ranked
}

fn is_tracker_domain(url: &CanonicalUrl) -> bool {
    url.0
        .host_str()
        .is_some_and(|host| TRACKER_DOMAINS.contains(host))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ranking::rank;
    use crate::{
        DeduplicatedResult, DuplicateStatus, OriginalUrl, Rank, RankingPolicyV1, ResultType,
        SCHEMA_VERSION,
    };
    use std::collections::BTreeMap;

    /// Builds a real `RankedResult` by running `rank()` on a single synthetic
    /// input, then rewrites its canonical URL host so a test can control
    /// whether it looks like a tracker domain. Reusing `rank()` keeps the
    /// `RankingExplanation` construction identical to production.
    fn ranked_with_host(host: &str) -> RankedResult {
        let url = url::Url::parse(&format!("https://{host}/page")).unwrap();
        let item = DeduplicatedResult {
            schema_version: SCHEMA_VERSION.into(),
            title: Some("example".into()),
            original_url: OriginalUrl(url.clone()),
            canonical_url: CanonicalUrl(url.clone()),
            original_urls: vec![OriginalUrl(url)],
            providers: vec!["p".into()],
            representative_provider: "p".into(),
            provider_ranks: BTreeMap::from([("p".to_string(), Rank::new(1).ok())]),
            snippet: Some("example snippet".into()),
            alternate_snippets: vec![],
            result_type: ResultType::Organic,
            published_at: None,
            author: None,
            language: None,
            file_type: None,
            thumbnail: None,
            metadata: BTreeMap::new(),
            observed_dates: vec![],
            duplicate_status: DuplicateStatus::Distinct,
            merge_reason: None,
            possible_duplicate_with: vec![],
        };
        let query = crate::parse_query("example".into()).unwrap();
        let mut ranked = rank(
            &query,
            "2026-08-12T00:00:00Z",
            1,
            vec![item],
            &RankingPolicyV1::default(),
        );
        ranked.remove(0)
    }

    fn with_host(mut r: RankedResult, host: &str) -> RankedResult {
        let url = url::Url::parse(&format!("https://{host}/page")).unwrap();
        r.result.canonical_url = CanonicalUrl(url);
        r
    }

    #[test]
    fn tracker_domain_score_is_multiplied_by_the_penalty() {
        // Pull a real tracker host straight out of the bundled list.
        let tracker_host = *TRACKER_DOMAINS.iter().next().expect("list is non-empty");
        let base = ranked_with_host("nontracker.example.org");
        let before = base.score.get();
        let adjusted = apply_adjustments(vec![with_host(base, tracker_host)]);
        let after = adjusted[0].score.get();
        assert_eq!(after, (before * TRACKER_PENALTY_MULTIPLIER).clamp(0.0, 1.0));
    }

    #[test]
    fn non_tracker_domain_score_is_unchanged() {
        let base = ranked_with_host("nontracker.example.org");
        let before = base.score.get();
        let adjusted = apply_adjustments(vec![base]);
        assert_eq!(adjusted[0].score.get(), before);
    }

    #[test]
    fn tracker_domain_list_is_not_empty() {
        assert!(
            !TRACKER_DOMAINS.is_empty(),
            "include_str! should have loaded the bundled tracker-domains.txt"
        );
    }
}
