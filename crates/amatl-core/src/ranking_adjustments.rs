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
//! Two adjustments live here today: an always-on tracker/ad-domain penalty and
//! the operator-declared Optics rules ([`crate::optics`]), passed in as an
//! optional parsed document. A future curated-independent-web bonus gets folded
//! into [`apply_adjustments`] the same way; `execution.rs` calls this one entry
//! point and only its argument list changes when a new source is added.

use crate::model::{CanonicalUrl, RankedResult, RankingScore, TieBreakReason};
use crate::optics::OpticsDocument;
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
/// the results are returned to the caller.
///
/// `optics` is the parsed [`OpticsDocument`] when `[ranking.optics]` is
/// enabled and its file parsed, `None` otherwise -- with `None` this behaves
/// exactly as it did before Optics existed (tracker penalty + re-sort only).
/// When present, `DiscardNonMatching` removes non-matching results here,
/// before the sort, and Boost/Downrank scale scores like the tracker penalty.
///
/// After the penalties are applied the `Vec` is re-sorted with the *exact*
/// same criterion `ranking::rank()` uses as its own final step (score desc,
/// then `title_match` desc, then `stable_order` asc), and each result's
/// `explanation.tie_break` is recomputed against its new neighbour -- for the
/// same reason: `rank()` set `tie_break` from a comparison it made *before*
/// this adjustment, which can be stale once a penalty changes the order.
/// Both steps mirror the tail of `rank()` verbatim; if that criterion ever
/// changes, this function must change with it.
pub fn apply_adjustments(
    mut ranked: Vec<RankedResult>,
    optics: Option<&OpticsDocument>,
) -> Vec<RankedResult> {
    for result in &mut ranked {
        if is_tracker_domain(&result.result.canonical_url) {
            let adjusted = (result.score.get() * TRACKER_PENALTY_MULTIPLIER).clamp(0.0, 1.0);
            result.score =
                RankingScore::new(adjusted).expect("clamped value is always within 0.0..=1.0");
        }
    }

    // Operator-declared Optics rules (crate::optics). Order relative to the
    // tracker penalty does not matter: both only scale scores, and the
    // re-sort below runs once, last. `DiscardNonMatching` is the one part
    // that removes entries rather than scaling them -- done here, before the
    // sort, so the dropped results never reach the caller at all.
    if let Some(document) = optics {
        if document.discard_non_matching {
            ranked.retain(|result| document.multiplier_for(result).matched);
        }
        for result in &mut ranked {
            let outcome = document.multiplier_for(result);
            if outcome.matched && outcome.multiplier != 1.0 {
                let adjusted = (result.score.get() * outcome.multiplier).clamp(0.0, 1.0);
                result.score =
                    RankingScore::new(adjusted).expect("clamped value is always within 0.0..=1.0");
            }
        }
    }

    ranked.sort_by(|left, right| {
        right
            .score
            .get()
            .total_cmp(&left.score.get())
            .then_with(|| right.title_match.get().total_cmp(&left.title_match.get()))
            .then_with(|| left.stable_order.cmp(&right.stable_order))
    });
    for index in 1..ranked.len() {
        ranked[index].explanation.tie_break = if ranked[index - 1].score == ranked[index].score {
            if ranked[index - 1].title_match == ranked[index].title_match {
                TieBreakReason::StableOrder
            } else {
                TieBreakReason::TitleMatch
            }
        } else {
            TieBreakReason::CombinedScore
        };
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
        let adjusted = apply_adjustments(vec![with_host(base, tracker_host)], None);
        let after = adjusted[0].score.get();
        assert_eq!(after, (before * TRACKER_PENALTY_MULTIPLIER).clamp(0.0, 1.0));
    }

    #[test]
    fn non_tracker_domain_score_is_unchanged() {
        let base = ranked_with_host("nontracker.example.org");
        let before = base.score.get();
        let adjusted = apply_adjustments(vec![base], None);
        assert_eq!(adjusted[0].score.get(), before);
    }

    #[test]
    fn tracker_domain_drops_below_a_non_tracker_result_after_adjustment() {
        let tracker_host = *TRACKER_DOMAINS.iter().next().expect("list is non-empty");

        // Two results that `rank()` scored identically (same synthetic input),
        // so score and title_match tie and `stable_order` alone decides the
        // order. Give the tracker result the lower `stable_order` so that
        // *before* the adjustment it sits first -- exactly the position the
        // penalty has to be able to take away from it.
        let mut tracker_first =
            with_host(ranked_with_host("tracker-holder.example.org"), tracker_host);
        tracker_first.stable_order = 0;
        let mut clean_second = ranked_with_host("clean.example.org");
        clean_second.stable_order = 1;

        // Pre-adjustment these two tie on score and title_match, so if `rank()`
        // had seen them as a pair it would have stamped the second one's
        // tie_break as StableOrder. Seed exactly that stale value and prove the
        // re-sort overwrites it once the penalty breaks the tie.
        tracker_first.explanation.tie_break = TieBreakReason::StableOrder;
        clean_second.explanation.tie_break = TieBreakReason::StableOrder;

        assert!(
            tracker_first.score.get() >= clean_second.score.get(),
            "test precondition: the tracker result must rank first pre-adjustment"
        );

        let adjusted = apply_adjustments(vec![tracker_first, clean_second], None);

        assert!(
            adjusted[0].result.canonical_url.0.host_str() != Some(tracker_host),
            "the tracker-domain result must not remain first after the penalty \
             reorders the list"
        );
        assert_eq!(
            adjusted[0].result.canonical_url.0.host_str(),
            Some("clean.example.org")
        );
        assert_eq!(
            adjusted[1].result.canonical_url.0.host_str(),
            Some(tracker_host)
        );
        // The penalty opened a real score gap between the two, so the demoted
        // tracker's tie_break must be recomputed to CombinedScore -- not left
        // at the stale StableOrder value seeded above.
        assert_eq!(
            adjusted[1].explanation.tie_break,
            TieBreakReason::CombinedScore
        );
    }

    #[test]
    fn tracker_domain_list_is_not_empty() {
        assert!(
            !TRACKER_DOMAINS.is_empty(),
            "include_str! should have loaded the bundled tracker-domains.txt"
        );
    }

    // ── Optics adjustments ───────────────────────────────────────────────

    use crate::optics::parse_optics;

    /// Two results `rank()` scored identically; `stable_order` decides. Give
    /// `first_host` the lower order so it leads pre-adjustment -- the position
    /// an optics rule must be able to change.
    fn two_tied(first_host: &str, second_host: &str) -> (RankedResult, RankedResult) {
        let mut first = with_host(ranked_with_host("holder-a.example.org"), first_host);
        first.stable_order = 0;
        let mut second = with_host(ranked_with_host("holder-b.example.org"), second_host);
        second.stable_order = 1;
        (first, second)
    }

    #[test]
    fn optics_boost_lifts_a_result_above_another_after_the_resort() {
        let (leader, challenger) = two_tied("plain.example.org", "docs.rs");
        assert!(leader.score.get() >= challenger.score.get());

        let optics =
            parse_optics(r#"Rule { Matches { Site("|docs.rs|") }, Action(Boost(5)) };"#).unwrap();
        let adjusted = apply_adjustments(vec![leader, challenger], Some(&optics));

        assert_eq!(
            adjusted[0].result.canonical_url.0.host_str(),
            Some("docs.rs"),
            "the boosted result must overtake the previously-leading one"
        );
    }

    #[test]
    fn optics_downrank_pushes_a_result_below_another_after_the_resort() {
        let (leader, challenger) = two_tied("spam.example.org", "clean.example.org");

        let optics = parse_optics(
            r#"Rule { Matches { Site("|spam.example.org|") }, Action(Downrank(5)) };"#,
        )
        .unwrap();
        let adjusted = apply_adjustments(vec![leader, challenger], Some(&optics));

        assert_eq!(
            adjusted[0].result.canonical_url.0.host_str(),
            Some("clean.example.org")
        );
        assert_eq!(
            adjusted[1].result.canonical_url.0.host_str(),
            Some("spam.example.org")
        );
    }

    #[test]
    fn discard_non_matching_removes_a_result_from_the_output_vec() {
        let keep = with_host(ranked_with_host("holder-a.example.org"), "docs.rs");
        let drop = with_host(
            ranked_with_host("holder-b.example.org"),
            "random.example.org",
        );

        let optics = parse_optics(
            r#"
            Rule { Matches { Site("|docs.rs|") }, Action(Boost(1)) };
            DiscardNonMatching;
        "#,
        )
        .unwrap();
        let adjusted = apply_adjustments(vec![keep, drop], Some(&optics));

        assert_eq!(adjusted.len(), 1, "the non-matching result must be gone");
        assert_eq!(
            adjusted[0].result.canonical_url.0.host_str(),
            Some("docs.rs")
        );
        assert!(
            adjusted
                .iter()
                .all(|r| r.result.canonical_url.0.host_str() != Some("random.example.org")),
            "a discarded result must not appear anywhere in the output, not merely rank last"
        );
    }

    #[test]
    fn none_optics_is_identical_to_pre_optics_behaviour() {
        // The exact tracker-sinkhole scenario from the test above, run with
        // `None` optics: behaviour must not have regressed.
        let tracker_host = *TRACKER_DOMAINS.iter().next().expect("list is non-empty");
        let mut tracker_first =
            with_host(ranked_with_host("tracker-holder.example.org"), tracker_host);
        tracker_first.stable_order = 0;
        let mut clean_second = ranked_with_host("clean.example.org");
        clean_second.stable_order = 1;

        let adjusted = apply_adjustments(vec![tracker_first, clean_second], None);

        assert_eq!(
            adjusted[0].result.canonical_url.0.host_str(),
            Some("clean.example.org")
        );
        assert_eq!(
            adjusted[1].result.canonical_url.0.host_str(),
            Some(tracker_host)
        );
    }

    #[test]
    fn empty_optics_document_changes_nothing() {
        let base = ranked_with_host("example.org");
        let before = base.score.get();
        let optics = parse_optics("// nothing here\n").unwrap();
        let adjusted = apply_adjustments(vec![base], Some(&optics));
        assert_eq!(adjusted[0].score.get(), before);
        assert_eq!(adjusted.len(), 1);
    }
}
