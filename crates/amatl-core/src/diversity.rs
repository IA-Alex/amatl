use crate::model::{
    Rank, RankedResult, RankingScore, ResultStatus, ResultType, SearchResult, SCHEMA_VERSION,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use thiserror::Error;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct DiversityPolicyV1 {
    pub version: String,
    pub max_visible_per_domain: usize,
    pub max_visible_per_provider: usize,
    pub max_visible_per_result_type: usize,
    pub relevance_override_ratio: f64,
}

impl Default for DiversityPolicyV1 {
    fn default() -> Self {
        Self {
            version: "v1".into(),
            max_visible_per_domain: 2,
            max_visible_per_provider: 5,
            max_visible_per_result_type: 6,
            relevance_override_ratio: 1.15,
        }
    }
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum DiversityPolicyError {
    #[error("diversity policy version must be v1")]
    Version,
    #[error("diversity limits must be positive")]
    Limit,
    #[error("diversity override ratio must be finite and at least one")]
    Override,
}

impl DiversityPolicyV1 {
    pub fn validate(&self) -> Result<(), DiversityPolicyError> {
        if self.version != "v1" {
            return Err(DiversityPolicyError::Version);
        }
        if self.max_visible_per_domain == 0
            || self.max_visible_per_provider == 0
            || self.max_visible_per_result_type == 0
        {
            return Err(DiversityPolicyError::Limit);
        }
        if !self.relevance_override_ratio.is_finite() || self.relevance_override_ratio < 1.0 {
            return Err(DiversityPolicyError::Override);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct DiversityDecision {
    pub rank: Rank,
    pub limit: String,
    pub group: String,
    pub score: RankingScore,
    pub comparator_score: Option<RankingScore>,
    pub override_applied: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DiversityOutput {
    pub results: Vec<SearchResult>,
    pub decisions: Vec<DiversityDecision>,
    pub metrics: DiversityMetrics,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DiversityMetrics {
    pub visible_results: usize,
    pub unique_domains: usize,
    pub unique_providers: usize,
    pub unique_result_types: usize,
}

pub fn diversify(results: Vec<RankedResult>, policy: &DiversityPolicyV1) -> DiversityOutput {
    let default_policy;
    let policy = if policy.validate().is_ok() {
        policy
    } else {
        default_policy = DiversityPolicyV1::default();
        &default_policy
    };
    let mut domains = BTreeMap::<String, usize>::new();
    let mut providers = BTreeMap::<String, usize>::new();
    let mut result_types = BTreeMap::<ResultType, usize>::new();
    let mut output = Vec::with_capacity(results.len());
    let mut decisions = Vec::new();
    for (index, ranked) in results.iter().enumerate() {
        let domain = ranked
            .result
            .canonical_url
            .0
            .host_str()
            .unwrap_or("")
            .to_string();
        let mut exceeded = Vec::new();
        if domains.get(&domain).copied().unwrap_or(0) >= policy.max_visible_per_domain {
            exceeded.push(Group::Domain(domain.clone()));
        }
        for provider in &ranked.result.providers {
            if providers.get(provider).copied().unwrap_or(0) >= policy.max_visible_per_provider {
                exceeded.push(Group::Provider(provider.clone()));
            }
        }
        if result_types
            .get(&ranked.result.result_type)
            .copied()
            .unwrap_or(0)
            >= policy.max_visible_per_result_type
        {
            exceeded.push(Group::ResultType(ranked.result.result_type.clone()));
        }
        let override_applied = !exceeded.is_empty()
            && exceeded.iter().all(|group| {
                next_comparator(&results, index, group).is_some_and(|score| {
                    ranked.score.get() >= score.get() * policy.relevance_override_ratio
                })
            });
        for group in &exceeded {
            decisions.push(DiversityDecision {
                rank: Rank::new(index as u32 + 1).unwrap_or(Rank::MAX),
                limit: group.limit_name().into(),
                group: group.value(),
                score: ranked.score,
                comparator_score: next_comparator(&results, index, group),
                override_applied,
            });
        }
        let visible = exceeded.is_empty() || override_applied;
        if visible {
            *domains.entry(domain.clone()).or_default() += 1;
            for provider in &ranked.result.providers {
                *providers.entry(provider.clone()).or_default() += 1;
            }
            *result_types
                .entry(ranked.result.result_type.clone())
                .or_default() += 1;
        }
        output.push(SearchResult {
            schema_version: SCHEMA_VERSION.into(),
            rank: Rank::new(index as u32 + 1).unwrap_or(Rank::MAX),
            title: ranked.result.title.clone(),
            original_url: ranked.result.original_url.clone(),
            canonical_url: ranked.result.canonical_url.clone(),
            domain,
            snippet: ranked.result.snippet.clone(),
            providers: ranked.result.providers.clone(),
            published_at: ranked.result.published_at.clone(),
            status: if visible {
                ResultStatus::Visible
            } else {
                ResultStatus::RelegatedByDiversity
            },
        });
    }
    DiversityOutput {
        results: output,
        decisions,
        metrics: DiversityMetrics {
            visible_results: domains.values().sum(),
            unique_domains: domains.len(),
            unique_providers: providers.len(),
            unique_result_types: result_types.len(),
        },
    }
}

#[derive(Clone, Debug)]
enum Group {
    Domain(String),
    Provider(String),
    ResultType(ResultType),
}

impl Group {
    fn limit_name(&self) -> &'static str {
        match self {
            Self::Domain(_) => "domain",
            Self::Provider(_) => "provider",
            Self::ResultType(_) => "result_type",
        }
    }
    fn value(&self) -> String {
        match self {
            Self::Domain(value) | Self::Provider(value) => value.clone(),
            Self::ResultType(value) => format!("{value:?}").to_ascii_lowercase(),
        }
    }
    fn matches(&self, candidate: &RankedResult) -> bool {
        match self {
            Self::Domain(domain) => candidate.result.canonical_url.0.host_str() == Some(domain),
            Self::Provider(provider) => candidate.result.providers.contains(provider),
            Self::ResultType(result_type) => &candidate.result.result_type == result_type,
        }
    }
}

fn next_comparator(results: &[RankedResult], index: usize, group: &Group) -> Option<RankingScore> {
    results[index + 1..]
        .iter()
        .find(|candidate| group.matches(candidate))
        .map(|candidate| candidate.score)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        CanonicalUrl, DeduplicatedResult, DuplicateStatus, OriginalUrl, RankingExplanation,
        TieBreakReason,
    };

    fn item(
        domain: &str,
        path: &str,
        score: f64,
        provider: &str,
        result_type: ResultType,
    ) -> RankedResult {
        let url = url::Url::parse(&format!("https://{domain}/{path}")).unwrap();
        let ranking_score = RankingScore::bounded(score);
        RankedResult {
            result: DeduplicatedResult {
                schema_version: SCHEMA_VERSION.into(),
                title: None,
                original_url: OriginalUrl(url.clone()),
                canonical_url: CanonicalUrl(url.clone()),
                original_urls: vec![OriginalUrl(url)],
                providers: vec![provider.into()],
                representative_provider: provider.into(),
                provider_ranks: Default::default(),
                snippet: None,
                alternate_snippets: vec![],
                result_type,
                published_at: None,
                author: None,
                language: None,
                file_type: None,
                thumbnail: None,
                metadata: Default::default(),
                observed_dates: vec![],
                duplicate_status: DuplicateStatus::Distinct,
                merge_reason: None,
                possible_duplicate_with: vec![],
            },
            score: ranking_score,
            title_match: RankingScore::bounded(0.0),
            stable_order: 0,
            explanation: RankingExplanation {
                ranking_policy: "v1".into(),
                rrf: RankingScore::bounded(0.0),
                title_match: RankingScore::bounded(0.0),
                snippet_match: RankingScore::bounded(0.0),
                freshness: RankingScore::bounded(0.0),
                provider_agreement: RankingScore::bounded(0.0),
                combined_score: ranking_score,
                tie_break: TieBreakReason::CombinedScore,
            },
        }
    }

    #[test]
    fn third_domain_result_is_relegated_without_deletion() {
        let output = diversify(
            vec![
                item("example.com", "a", 0.9, "p", ResultType::Organic),
                item("example.com", "b", 0.8, "p", ResultType::Organic),
                item("example.com", "c", 0.7, "p", ResultType::Organic),
            ],
            &DiversityPolicyV1::default(),
        );
        assert_eq!(output.results.len(), 3);
        assert_eq!(output.results[2].status, ResultStatus::RelegatedByDiversity);
        assert!(!output.decisions[0].override_applied);
    }

    #[test]
    fn strong_candidate_can_override_when_a_comparator_exists() {
        let output = diversify(
            vec![
                item("example.com", "a", 1.0, "a", ResultType::Organic),
                item("example.com", "b", 0.9, "b", ResultType::News),
                item("example.com", "c", 0.8, "c", ResultType::Media),
                item("example.com", "d", 0.5, "d", ResultType::Document),
            ],
            &DiversityPolicyV1::default(),
        );
        assert_eq!(output.results[2].status, ResultStatus::Visible);
        assert!(output.decisions[0].override_applied);
    }
}
