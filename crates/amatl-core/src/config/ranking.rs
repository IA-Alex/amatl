use super::*;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct RankingV2Config {
    pub enabled: bool,
    pub policy: RankingV2Policy,
}

/// `[ranking]` — post-rank adjustments that live outside the protected
/// `RankingPolicyV1` weighted-sum formula (`crate::ranking_adjustments`).
///
/// Today this holds only the Optics sub-section; the tracker/ad-domain
/// penalty is always on and has no knobs.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct RankingConfig {
    pub optics: OpticsConfig,
}

/// `[ranking.optics]` — operator-declared Boost/Downrank/Discard rules loaded
/// from a static `.optic` file (see `crate::optics`). Rules are loaded once,
/// from a file, exactly like every other AMATL setting — never per request, so
/// the search API is not a way to inject ranking rules.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct OpticsConfig {
    /// When `false` (the default) the `.optic` file, if any, is ignored.
    pub enabled: bool,
    /// Path to the `.optic` file, resolved relative to the process working
    /// directory. A missing or unparsable file logs a warning and disables
    /// optics for that load — it never fails a search.
    pub path: Option<String>,
}

/// Narrow admin-scoped patch for `[ranking.optics]`. Mirrors
/// [`Config::set_ranking_optics_fields`] exactly.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct OpticsConfigPatch {
    pub enabled: Option<bool>,
    pub path: Option<String>,
}

impl OpticsConfigPatch {
    /// Apply every field this patch sets to `config`. Mirrors
    /// [`Config::set_ranking_optics_fields`].
    pub fn apply(&self, config: &mut OpticsConfig) {
        if let Some(value) = self.enabled {
            config.enabled = value;
        }
        if let Some(value) = &self.path {
            config.path = Some(value.clone());
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct GapConfig {
    pub enabled: bool,
    pub max_subqueries: u32,
    pub max_cost: u64,
    pub max_provider_calls_per_subquery: u32,
    pub timeout_ms: u64,
    pub policy: GapPolicyV1,
}

impl Default for RankingV2Config {
    fn default() -> Self {
        Self {
            enabled: true,
            policy: RankingV2Policy::default(),
        }
    }
}
impl Default for GapConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_subqueries: 2,
            max_cost: 2,
            max_provider_calls_per_subquery: 2,
            timeout_ms: 5_000,
            policy: GapPolicyV1::default(),
        }
    }
}

impl Config {
    /// Write every field `patch` sets to `[ranking.optics]`, and nothing
    /// else. Same guarantees as [`Config::set_deep_fields`]: uses `toml_edit`
    /// so operator comments survive, and the caller is expected to have
    /// validated a candidate configuration first.
    pub fn set_ranking_optics_fields(
        path: &Path,
        patch: &OpticsConfigPatch,
    ) -> Result<(), ConfigError> {
        let text = std::fs::read_to_string(path)?;
        let mut document = text
            .parse::<toml_edit::DocumentMut>()
            .map_err(|error| ConfigError::Policy(format!("invalid TOML: {error}")))?;
        let table = Self::table_at(&mut document, &["ranking", "optics"])?;
        if let Some(value) = patch.enabled {
            table.insert("enabled", toml_edit::value(value));
        }
        if let Some(value) = &patch.path {
            table.insert("path", toml_edit::value(value.as_str()));
        }
        write_config_document(path, &document)?;
        Ok(())
    }

    /// Replace `[ranking_policy]` wholesale with `policy`, and nothing else.
    /// See `Config::replace_table` for what "wholesale" means here.
    pub fn set_ranking_policy(path: &Path, policy: &RankingPolicyV1) -> Result<(), ConfigError> {
        Self::replace_table(path, &["ranking_policy"], policy)
    }

    /// Replace `[diversity_policy]` wholesale. See
    /// [`Config::set_ranking_policy`].
    ///
    /// The caller must validate a candidate first: `diversity_policy` and
    /// `search_policy` share three limits that `Config::validate` requires
    /// to agree (see the cross-check there), so replacing one alone can
    /// make the pair invalid even though each is individually well-formed.
    pub fn set_diversity_policy(
        path: &Path,
        policy: &DiversityPolicyV1,
    ) -> Result<(), ConfigError> {
        Self::replace_table(path, &["diversity_policy"], policy)
    }

    /// Replace `[search_policy]` wholesale. See
    /// [`Config::set_diversity_policy`] for the cross-check with
    /// `diversity_policy` this must still satisfy.
    pub fn set_search_policy(path: &Path, policy: &SearchPolicyV1) -> Result<(), ConfigError> {
        Self::replace_table(path, &["search_policy"], policy)
    }

    /// Replace `[deep.ranking_v2.policy]` wholesale. See
    /// [`Config::set_ranking_policy`].
    pub fn set_ranking_v2_policy(path: &Path, policy: &RankingV2Policy) -> Result<(), ConfigError> {
        Self::replace_table(path, &["deep", "ranking_v2", "policy"], policy)
    }

    /// Replace `[deep.gaps.policy]` wholesale. See
    /// [`Config::set_ranking_policy`].
    pub fn set_gap_policy(path: &Path, policy: &GapPolicyV1) -> Result<(), ConfigError> {
        Self::replace_table(path, &["deep", "gaps", "policy"], policy)
    }

    /// The three result-shaping sub-policies and the requirement that the
    /// visible-per-domain / -provider / -result-type caps agree between the
    /// search policy and the diversity policy.
    pub(super) fn validate_ranking_policies(&self) -> Result<(), ConfigError> {
        self.ranking_policy
            .validate()
            .map_err(|error| ConfigError::Policy(error.to_string()))?;
        self.diversity_policy
            .validate()
            .map_err(|error| ConfigError::Policy(error.to_string()))?;
        self.search_policy
            .validate()
            .map_err(|error| ConfigError::Policy(error.to_string()))?;
        if self.search_policy.maximum_results_per_domain
            != self.diversity_policy.max_visible_per_domain
            || self.search_policy.maximum_results_per_provider
                != self.diversity_policy.max_visible_per_provider
            || self.search_policy.maximum_results_per_result_type
                != self.diversity_policy.max_visible_per_result_type
        {
            return Err(ConfigError::Policy(
                "search policy and diversity policy limits must agree".into(),
            ));
        }
        Ok(())
    }
}
