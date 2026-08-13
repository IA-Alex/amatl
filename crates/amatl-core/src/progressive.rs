use crate::diversity::DiversityMetrics;
use crate::model::{ResultStatus, SearchResult};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct SearchPolicyV1 {
    pub version: String,
    pub first_round_min_providers: usize,
    pub first_round_max_providers: usize,
    pub minimum_useful_results: usize,
    pub target_useful_results: usize,
    pub minimum_unique_domains: usize,
    pub target_unique_domains: usize,
    pub low_diversity_domain_ratio: f64,
    pub low_diversity_provider_ratio: f64,
    pub low_diversity_result_type_ratio: f64,
    pub minimum_marginal_gain: f64,
    pub minimum_expected_marginal_gain: f64,
    pub minimum_remaining_deadline_ms: u64,
    pub maximum_results_per_domain: usize,
    pub maximum_results_per_provider: usize,
    pub maximum_results_per_result_type: usize,
    pub minimum_exploration_ratio: f64,
}

impl Default for SearchPolicyV1 {
    fn default() -> Self {
        Self {
            version: "v1".into(),
            first_round_min_providers: 2,
            first_round_max_providers: 3,
            minimum_useful_results: 8,
            target_useful_results: 12,
            minimum_unique_domains: 4,
            target_unique_domains: 6,
            low_diversity_domain_ratio: 0.50,
            low_diversity_provider_ratio: 0.20,
            low_diversity_result_type_ratio: 0.20,
            minimum_marginal_gain: 0.15,
            minimum_expected_marginal_gain: 0.15,
            minimum_remaining_deadline_ms: 750,
            maximum_results_per_domain: 2,
            maximum_results_per_provider: 5,
            maximum_results_per_result_type: 6,
            minimum_exploration_ratio: 0.10,
        }
    }
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum SearchPolicyError {
    #[error("search policy version must be v1")]
    Version,
    #[error("search policy counts and limits must be positive and ordered")]
    Limit,
    #[error("search policy ratios must be finite values between zero and one")]
    Ratio,
}

impl SearchPolicyV1 {
    pub fn validate(&self) -> Result<(), SearchPolicyError> {
        if self.version != "v1" {
            return Err(SearchPolicyError::Version);
        }
        if self.first_round_min_providers == 0
            || self.first_round_max_providers < self.first_round_min_providers
            || self.minimum_useful_results == 0
            || self.target_useful_results < self.minimum_useful_results
            || self.minimum_unique_domains == 0
            || self.target_unique_domains < self.minimum_unique_domains
            || self.minimum_remaining_deadline_ms == 0
            || self.maximum_results_per_domain == 0
            || self.maximum_results_per_provider == 0
            || self.maximum_results_per_result_type == 0
        {
            return Err(SearchPolicyError::Limit);
        }
        let ratios = [
            self.low_diversity_domain_ratio,
            self.low_diversity_provider_ratio,
            self.low_diversity_result_type_ratio,
            self.minimum_marginal_gain,
            self.minimum_expected_marginal_gain,
            self.minimum_exploration_ratio,
        ];
        if ratios
            .iter()
            .any(|value| !value.is_finite() || !(0.0..=1.0).contains(value))
        {
            return Err(SearchPolicyError::Ratio);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CoverageMetrics {
    pub useful_results: usize,
    pub unique_domains: usize,
    pub unique_providers: usize,
    pub unique_result_types: usize,
    pub visible_results: usize,
    pub coverage_minimum: bool,
    pub coverage_target: bool,
    pub low_diversity: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SearchStopReason {
    TimeExhausted,
    DeadlineNear,
    ProviderLimit,
    CoverageTargetReached,
    MarginalGainLow,
    ProvidersExhausted,
    ExplicitFilterSatisfied,
}

impl SearchStopReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::TimeExhausted => "time_exhausted",
            Self::DeadlineNear => "deadline_near",
            Self::ProviderLimit => "provider_limit",
            Self::CoverageTargetReached => "coverage_target_reached",
            Self::MarginalGainLow => "marginal_gain_low",
            Self::ProvidersExhausted => "providers_exhausted",
            Self::ExplicitFilterSatisfied => "explicit_filter_satisfied",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProgressiveRoundTrace {
    pub round: u32,
    pub providers_considered: Vec<String>,
    pub providers_selected: Vec<String>,
    pub providers_skipped: Vec<String>,
    pub useful_results: usize,
    pub unique_domains: usize,
    pub unique_providers: usize,
    pub unique_result_types: usize,
    pub coverage_minimum: bool,
    pub coverage_target: bool,
    pub low_diversity: bool,
    pub expected_marginal_gain_by_provider: BTreeMap<String, f64>,
    pub observed_marginal_gain: Option<f64>,
    pub stop_reason: Option<SearchStopReason>,
    pub debug_reasons: Vec<String>,
}

pub fn evaluate_coverage(
    results: &[SearchResult],
    diversity: &DiversityMetrics,
    policy: &SearchPolicyV1,
) -> CoverageMetrics {
    let default_policy;
    let policy = if policy.validate().is_ok() {
        policy
    } else {
        default_policy = SearchPolicyV1::default();
        &default_policy
    };
    let useful = results
        .iter()
        .filter(|result| {
            matches!(
                result.status,
                ResultStatus::Visible | ResultStatus::RelegatedByDiversity
            ) && !result.domain.is_empty()
        })
        .collect::<Vec<_>>();
    let unique_domains = useful
        .iter()
        .map(|result| result.domain.as_str())
        .collect::<BTreeSet<_>>()
        .len();
    let unique_providers = useful
        .iter()
        .flat_map(|result| result.providers.iter().map(String::as_str))
        .collect::<BTreeSet<_>>()
        .len();
    let useful_results = useful.len();
    let coverage_minimum = useful_results >= policy.minimum_useful_results
        && unique_domains >= policy.minimum_unique_domains;
    let coverage_target = useful_results >= policy.target_useful_results
        && unique_domains >= policy.target_unique_domains;
    let visible = diversity.visible_results;
    let low_diversity = if visible < 3 {
        false
    } else {
        diversity.unique_domains as f64 / (visible as f64) < policy.low_diversity_domain_ratio
            || diversity.unique_providers as f64 / (visible as f64)
                < policy.low_diversity_provider_ratio
            || diversity.unique_result_types as f64 / (visible as f64)
                < policy.low_diversity_result_type_ratio
    };
    CoverageMetrics {
        useful_results,
        unique_domains,
        unique_providers,
        unique_result_types: diversity.unique_result_types,
        visible_results: visible,
        coverage_minimum,
        coverage_target,
        low_diversity,
    }
}

pub fn observed_marginal_gain(previous: &[SearchResult], current: &[SearchResult]) -> f64 {
    let before = previous
        .iter()
        .map(|result| result.canonical_url.0.as_str())
        .collect::<BTreeSet<_>>();
    current
        .iter()
        .filter(|result| !before.contains(result.canonical_url.0.as_str()))
        .count() as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policy_defaults_are_valid_and_versioned() {
        assert!(SearchPolicyV1::default().validate().is_ok());
        let invalid = SearchPolicyV1 {
            version: "v2".into(),
            ..SearchPolicyV1::default()
        };
        assert_eq!(invalid.validate(), Err(SearchPolicyError::Version));
    }
}
