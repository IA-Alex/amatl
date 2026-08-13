use crate::model::{
    DeduplicatedResult, Query, RankedResult, RankingExplanation, RankingScore, TieBreakReason,
};
use crate::text::{normalized_text, tokens};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use thiserror::Error;
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct RankingPolicyV1 {
    pub version: String,
    pub rrf_k: u32,
    pub weight_rrf: f64,
    pub weight_title_match: f64,
    pub weight_snippet_match: f64,
    pub weight_freshness: f64,
    pub weight_provider_agreement: f64,
    pub freshness_half_life_days: u32,
    pub freshness_unknown: f64,
}

impl Default for RankingPolicyV1 {
    fn default() -> Self {
        Self {
            version: "v1".into(),
            rrf_k: 60,
            weight_rrf: 0.35,
            weight_title_match: 0.30,
            weight_snippet_match: 0.15,
            weight_freshness: 0.10,
            weight_provider_agreement: 0.10,
            freshness_half_life_days: 30,
            freshness_unknown: 0.0,
        }
    }
}

#[derive(Clone, Debug, Error, PartialEq)]
pub enum RankingPolicyError {
    #[error("ranking policy version must be v1")]
    Version,
    #[error("rrf_k and freshness_half_life_days must be positive")]
    PositiveInteger,
    #[error("ranking weights and freshness_unknown must be in range")]
    Range,
    #[error("ranking weights must sum to one")]
    WeightSum,
}

impl RankingPolicyV1 {
    pub fn validate(&self) -> Result<(), RankingPolicyError> {
        if self.version != "v1" {
            return Err(RankingPolicyError::Version);
        }
        if self.rrf_k == 0 || self.freshness_half_life_days == 0 {
            return Err(RankingPolicyError::PositiveInteger);
        }
        let values = [
            self.weight_rrf,
            self.weight_title_match,
            self.weight_snippet_match,
            self.weight_freshness,
            self.weight_provider_agreement,
            self.freshness_unknown,
        ];
        if values.iter().any(|value| !(0.0..=1.0).contains(value)) {
            return Err(RankingPolicyError::Range);
        }
        let sum = self.weight_rrf
            + self.weight_title_match
            + self.weight_snippet_match
            + self.weight_freshness
            + self.weight_provider_agreement;
        if (sum - 1.0).abs() > 1e-12 {
            return Err(RankingPolicyError::WeightSum);
        }
        Ok(())
    }
}

pub fn rank(
    query: &Query,
    ranking_reference_time: &str,
    active_provider_count: usize,
    mut results: Vec<DeduplicatedResult>,
    policy: &RankingPolicyV1,
) -> Vec<RankedResult> {
    let default_policy;
    let policy = if policy.validate().is_ok() {
        policy
    } else {
        default_policy = RankingPolicyV1::default();
        &default_policy
    };
    results.sort_by(|left, right| {
        left.canonical_url
            .cmp(&right.canonical_url)
            .then_with(|| left.original_url.cmp(&right.original_url))
            .then_with(|| {
                left.representative_provider
                    .cmp(&right.representative_provider)
            })
    });
    let reference = OffsetDateTime::parse(ranking_reference_time, &Rfc3339).ok();
    let query_tokens = ranking_query_tokens(query);
    let phrases = query
        .quoted_terms
        .iter()
        .map(|phrase| normalized_text(phrase))
        .filter(|phrase| !phrase.is_empty())
        .collect::<Vec<_>>();
    let mut ranked = results
        .into_iter()
        .enumerate()
        .map(|(stable_order, result)| {
            let rrf = rrf(&result, policy.rrf_k);
            let title_match = text_match(result.title.as_deref(), &query_tokens, &phrases);
            let snippet_match = text_match(result.snippet.as_deref(), &query_tokens, &phrases);
            let freshness = freshness(result.published_at.as_deref(), reference, policy);
            let provider_agreement = agreement(result.providers.len(), active_provider_count);
            let combined = (policy.weight_rrf * rrf
                + policy.weight_title_match * title_match
                + policy.weight_snippet_match * snippet_match
                + policy.weight_freshness * freshness
                + policy.weight_provider_agreement * provider_agreement)
                .clamp(0.0, 1.0);
            RankedResult {
                result,
                score: RankingScore::bounded(combined),
                title_match: RankingScore::bounded(title_match),
                stable_order,
                explanation: RankingExplanation {
                    ranking_policy: policy.version.clone(),
                    rrf: RankingScore::bounded(rrf),
                    title_match: RankingScore::bounded(title_match),
                    snippet_match: RankingScore::bounded(snippet_match),
                    freshness: RankingScore::bounded(freshness),
                    provider_agreement: RankingScore::bounded(provider_agreement),
                    combined_score: RankingScore::bounded(combined),
                    tie_break: TieBreakReason::CombinedScore,
                },
            }
        })
        .collect::<Vec<_>>();
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

fn ranking_query_tokens(query: &Query) -> BTreeSet<String> {
    let mut output = tokens(&query.normalized_query);
    for phrase in &query.quoted_terms {
        output.extend(tokens(phrase));
    }
    output
}

fn rrf(result: &DeduplicatedResult, k: u32) -> f64 {
    let ranks = result
        .provider_ranks
        .values()
        .filter_map(|rank| *rank)
        .collect::<Vec<_>>();
    if ranks.is_empty() {
        return 0.0;
    }
    let raw = ranks
        .iter()
        .map(|rank| 1.0 / (k as f64 + rank.get() as f64))
        .sum::<f64>();
    (raw / ranks.len() as f64 / (1.0 / (k as f64 + 1.0))).clamp(0.0, 1.0)
}

fn text_match(value: Option<&str>, query_tokens: &BTreeSet<String>, phrases: &[String]) -> f64 {
    let Some(value) = value else { return 0.0 };
    if query_tokens.is_empty() {
        return 0.0;
    }
    let prepared = normalized_text(value);
    let field_tokens = tokens(&prepared);
    let coverage =
        query_tokens.intersection(&field_tokens).count() as f64 / query_tokens.len() as f64;
    let phrase_bonus = if phrases.is_empty() {
        0.0
    } else {
        phrases
            .iter()
            .filter(|phrase| prepared.contains(phrase.as_str()))
            .count() as f64
            / phrases.len() as f64
    };
    (0.85 * coverage + 0.15 * phrase_bonus).min(1.0)
}

fn freshness(
    published_at: Option<&str>,
    reference: Option<OffsetDateTime>,
    policy: &RankingPolicyV1,
) -> f64 {
    let Some(reference) = reference else {
        return policy.freshness_unknown;
    };
    let Some(published) =
        published_at.and_then(|value| OffsetDateTime::parse(value, &Rfc3339).ok())
    else {
        return policy.freshness_unknown;
    };
    let age_seconds = (reference - published).whole_seconds().max(0) as f64;
    let age_days = age_seconds / 86_400.0;
    2_f64.powf(-age_days / policy.freshness_half_life_days as f64)
}

fn agreement(provider_count: usize, active_provider_count: usize) -> f64 {
    if active_provider_count <= 1 {
        0.0
    } else {
        (provider_count.saturating_sub(1) as f64 / (active_provider_count - 1) as f64)
            .clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CanonicalUrl, DuplicateStatus, OriginalUrl, ResultType, SCHEMA_VERSION};
    use std::collections::BTreeMap;

    fn item(
        title: Option<&str>,
        snippet: Option<&str>,
        path: &str,
        ranks: &[(&str, Option<u32>)],
    ) -> DeduplicatedResult {
        let url = url::Url::parse(&format!("https://example.com/{path}")).unwrap();
        DeduplicatedResult {
            schema_version: SCHEMA_VERSION.into(),
            title: title.map(str::to_string),
            original_url: OriginalUrl(url.clone()),
            canonical_url: CanonicalUrl(url.clone()),
            original_urls: vec![OriginalUrl(url)],
            providers: ranks.iter().map(|(p, _)| (*p).into()).collect(),
            representative_provider: ranks[0].0.into(),
            provider_ranks: ranks
                .iter()
                .map(|(p, rank)| {
                    (
                        (*p).into(),
                        rank.and_then(|value| crate::Rank::new(value).ok()),
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
            duplicate_status: DuplicateStatus::Distinct,
            merge_reason: None,
            possible_duplicate_with: vec![],
        }
    }

    #[test]
    fn policy_values_are_calibrable_within_contract_invariants() {
        assert_eq!(RankingPolicyV1::default().validate(), Ok(()));
        let changed = RankingPolicyV1 {
            rrf_k: 61,
            ..RankingPolicyV1::default()
        };
        assert_eq!(changed.validate(), Ok(()));
    }

    #[test]
    fn rrf_rank_one_is_one_and_missing_rank_is_zero() {
        assert_eq!(rrf(&item(Some("x"), None, "a", &[("p", Some(1))]), 60), 1.0);
        assert_eq!(rrf(&item(Some("x"), None, "b", &[("p", None)]), 60), 0.0);
    }

    #[test]
    fn snippet_contributes_without_becoming_tie_break() {
        let query = crate::parse_query("rust async".into()).unwrap();
        let ranked = rank(
            &query,
            "2026-08-12T00:00:00Z",
            2,
            vec![
                item(None, Some("rust async guide"), "b", &[("p", None)]),
                item(Some("unrelated"), None, "a", &[("p", None)]),
            ],
            &RankingPolicyV1::default(),
        );
        assert_eq!(ranked[0].result.canonical_url.0.path(), "/b");
        assert_eq!(ranked[0].explanation.title_match.get(), 0.0);
        assert!(ranked[0].explanation.snippet_match.get() > 0.0);
    }

    #[test]
    fn identical_inputs_are_reproducible() {
        let query = crate::parse_query("rust".into()).unwrap();
        let input = vec![
            item(Some("rust"), None, "b", &[("p", Some(1))]),
            item(Some("rust"), None, "a", &[("p", Some(1))]),
        ];
        let first = rank(
            &query,
            "2026-08-12T00:00:00Z",
            1,
            input.clone(),
            &RankingPolicyV1::default(),
        );
        let second = rank(
            &query,
            "2026-08-12T00:00:00Z",
            1,
            input,
            &RankingPolicyV1::default(),
        );
        assert_eq!(first, second);
        assert_eq!(first[0].result.canonical_url.0.path(), "/a");
        assert_eq!(first[1].explanation.tie_break, TieBreakReason::StableOrder);
    }

    #[test]
    fn agreement_and_rrf_use_only_valid_contributing_providers() {
        let query = crate::parse_query("rust".into()).unwrap();
        let ranked = rank(
            &query,
            "2026-08-12T00:00:00Z",
            2,
            vec![item(
                Some("rust"),
                None,
                "a",
                &[("a", Some(1)), ("b", Some(1))],
            )],
            &RankingPolicyV1::default(),
        );
        assert_eq!(ranked[0].explanation.rrf.get(), 1.0);
        assert_eq!(ranked[0].explanation.provider_agreement.get(), 1.0);
    }

    #[test]
    fn freshness_is_zero_when_unknown_and_one_for_future_dates() {
        let policy = RankingPolicyV1::default();
        assert_eq!(
            freshness(
                None,
                OffsetDateTime::parse("2026-08-12T00:00:00Z", &Rfc3339).ok(),
                &policy
            ),
            0.0
        );
        assert_eq!(
            freshness(
                Some("2026-08-13T00:00:00Z"),
                OffsetDateTime::parse("2026-08-12T00:00:00Z", &Rfc3339).ok(),
                &policy
            ),
            1.0
        );
    }
}
