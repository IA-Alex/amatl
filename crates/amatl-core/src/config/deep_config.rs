use super::*;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct DeepConfig {
    pub top_k: u32,
    pub max_fetches: u32,
    pub max_bytes: u64,
    pub max_redirects: u32,
    pub max_crawl_urls: u32,
    pub max_depth: u8,
    /// Consult `robots.txt` before fetching a link discovered by the crawl.
    /// URLs that came from Search are requested by the user and are not gated.
    pub respect_robots: bool,
    /// Deadline for one `robots.txt` retrieval.
    pub robots_timeout_ms: u64,
    /// Byte ceiling for one `robots.txt` retrieval.
    pub robots_max_bytes: u64,
    pub timeout_ms: u64,
    pub extractor: ExtractorConfig,
    pub renderer: RendererConfig,
    pub ranking_v2: RankingV2Config,
    pub gaps: GapConfig,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ExtractorConfig {
    pub executable: String,
    pub version: String,
    pub timeout_ms: u64,
    pub max_output_bytes: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct RendererConfig {
    pub enabled: bool,
    pub max_browser_calls: u32,
    pub timeout_ms: u64,
    pub shutdown_grace_ms: u64,
    pub max_memory_mb: u64,
    /// Redirect ceiling inherited by the fetch that produced the DOM.
    ///
    /// The renderer itself never navigates — it is handed bytes that
    /// [`crate::SafeFetcher`] already retrieved — so it contributes no
    /// redirects of its own.
    pub max_redirects: u32,
    /// Path to the `amatl-chromium-sandbox` isolation harness.
    ///
    /// Chromium is never launched directly: the harness is the only supported
    /// entry point because it is what enforces the network, memory, task and
    /// filesystem confinement the renderer's safety argument rests on.
    pub sandbox_path: String,
    /// Largest DOM the harness may return, in bytes.
    pub max_dom_bytes: u64,
}

impl Default for DeepConfig {
    fn default() -> Self {
        Self {
            top_k: 5,
            max_fetches: 10,
            max_bytes: 20 * 1024 * 1024,
            max_redirects: 5,
            max_crawl_urls: 10,
            max_depth: 1,
            respect_robots: true,
            robots_timeout_ms: 3_000,
            robots_max_bytes: 512 * 1024,
            timeout_ms: 20_000,
            extractor: ExtractorConfig::default(),
            renderer: RendererConfig::default(),
            ranking_v2: RankingV2Config::default(),
            gaps: GapConfig::default(),
        }
    }
}
impl Default for ExtractorConfig {
    fn default() -> Self {
        Self {
            executable: "trafilatura".into(),
            version: "trafilatura-2.2.0-cli-json-v1".into(),
            timeout_ms: 8_000,
            max_output_bytes: 4 * 1024 * 1024,
        }
    }
}

impl Default for RendererConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            max_browser_calls: 2,
            timeout_ms: 8_000,
            shutdown_grace_ms: 500,
            max_memory_mb: 512,
            max_redirects: 5,
            sandbox_path: "amatl-chromium-sandbox".into(),
            max_dom_bytes: 8 * 1024 * 1024,
        }
    }
}
/// The [`DeepConfig`] top-level safety limits an admin-scoped caller may
/// update in one request. Deliberately excludes `extractor`, `renderer`,
/// `ranking_v2` and `gaps` — each of those is its own nested table with its
/// own patch type ([`ExtractorConfigPatch`], [`RendererConfigPatch`]) or, for
/// `ranking_v2.policy`/`gaps.policy`, its own `/policies/*` endpoint (see
/// `Config::set_ranking_v2_policy`/`set_gap_policy`), rather than being
/// folded into this one.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct DeepConfigPatch {
    pub top_k: Option<u32>,
    pub max_fetches: Option<u32>,
    pub max_bytes: Option<u64>,
    pub max_redirects: Option<u32>,
    pub max_crawl_urls: Option<u32>,
    pub max_depth: Option<u8>,
    pub respect_robots: Option<bool>,
    pub robots_timeout_ms: Option<u64>,
    pub robots_max_bytes: Option<u64>,
    pub timeout_ms: Option<u64>,
}

impl DeepConfigPatch {
    /// Apply every field this patch sets to `config`, leaving `extractor`,
    /// `renderer`, `ranking_v2` and `gaps` untouched. Mirrors
    /// [`Config::set_deep_fields`] exactly.
    pub fn apply(&self, config: &mut DeepConfig) {
        if let Some(value) = self.top_k {
            config.top_k = value;
        }
        if let Some(value) = self.max_fetches {
            config.max_fetches = value;
        }
        if let Some(value) = self.max_bytes {
            config.max_bytes = value;
        }
        if let Some(value) = self.max_redirects {
            config.max_redirects = value;
        }
        if let Some(value) = self.max_crawl_urls {
            config.max_crawl_urls = value;
        }
        if let Some(value) = self.max_depth {
            config.max_depth = value;
        }
        if let Some(value) = self.respect_robots {
            config.respect_robots = value;
        }
        if let Some(value) = self.robots_timeout_ms {
            config.robots_timeout_ms = value;
        }
        if let Some(value) = self.robots_max_bytes {
            config.robots_max_bytes = value;
        }
        if let Some(value) = self.timeout_ms {
            config.timeout_ms = value;
        }
    }
}

/// Every [`ExtractorConfig`] field an admin-scoped caller may update.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct ExtractorConfigPatch {
    pub executable: Option<String>,
    pub version: Option<String>,
    pub timeout_ms: Option<u64>,
    pub max_output_bytes: Option<u64>,
}

impl ExtractorConfigPatch {
    /// Apply every field this patch sets to `config`. Mirrors
    /// [`Config::set_deep_extractor_fields`] exactly. Unlike the
    /// `Option<String>` fields on [`InferenceConfigPatch`], `executable` and
    /// `version` are themselves required (non-optional) strings in
    /// [`ExtractorConfig`], so there is no "clear" case to support here —
    /// `Some(value)` always sets a real replacement.
    pub fn apply(&self, config: &mut ExtractorConfig) {
        if let Some(value) = &self.executable {
            config.executable = value.clone();
        }
        if let Some(value) = &self.version {
            config.version = value.clone();
        }
        if let Some(value) = self.timeout_ms {
            config.timeout_ms = value;
        }
        if let Some(value) = self.max_output_bytes {
            config.max_output_bytes = value;
        }
    }
}

/// Every [`RendererConfig`] field an admin-scoped caller may update.
///
/// Setting `enabled = true` while `data_policy.profile = "isolated"` fails
/// `Config::validate` (an isolated profile forbids the unsandboxed
/// renderer), so a candidate built with this patch and validated before
/// writing already fails closed on that combination — see
/// [`Config::set_deep_renderer_fields`].
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct RendererConfigPatch {
    pub enabled: Option<bool>,
    pub max_browser_calls: Option<u32>,
    pub timeout_ms: Option<u64>,
    pub shutdown_grace_ms: Option<u64>,
    pub max_memory_mb: Option<u64>,
    pub max_redirects: Option<u32>,
    pub sandbox_path: Option<String>,
    pub max_dom_bytes: Option<u64>,
}

impl RendererConfigPatch {
    /// Apply every field this patch sets to `config`. Mirrors
    /// [`Config::set_deep_renderer_fields`] exactly.
    pub fn apply(&self, config: &mut RendererConfig) {
        if let Some(value) = self.enabled {
            config.enabled = value;
        }
        if let Some(value) = self.max_browser_calls {
            config.max_browser_calls = value;
        }
        if let Some(value) = self.timeout_ms {
            config.timeout_ms = value;
        }
        if let Some(value) = self.shutdown_grace_ms {
            config.shutdown_grace_ms = value;
        }
        if let Some(value) = self.max_memory_mb {
            config.max_memory_mb = value;
        }
        if let Some(value) = self.max_redirects {
            config.max_redirects = value;
        }
        if let Some(value) = &self.sandbox_path {
            config.sandbox_path = value.clone();
        }
        if let Some(value) = self.max_dom_bytes {
            config.max_dom_bytes = value;
        }
    }
}

impl Config {
    /// Write every field `patch` sets to `[deep]`, and nothing else —
    /// `extractor`, `renderer`, `ranking_v2` and `gaps` are separate nested
    /// tables this never touches (see [`DeepConfigPatch`]). Same guarantees
    /// as [`Config::set_inference_fields`].
    pub fn set_deep_fields(path: &Path, patch: &DeepConfigPatch) -> Result<(), ConfigError> {
        let text = std::fs::read_to_string(path)?;
        let mut document = text
            .parse::<toml_edit::DocumentMut>()
            .map_err(|error| ConfigError::Policy(format!("invalid TOML: {error}")))?;
        let table = Self::table_at(&mut document, &["deep"])?;
        if let Some(value) = patch.top_k {
            table.insert(
                "top_k",
                toml_edit::value(signed_integer(value, "deep.top_k")?),
            );
        }
        if let Some(value) = patch.max_fetches {
            table.insert(
                "max_fetches",
                toml_edit::value(signed_integer(value, "deep.max_fetches")?),
            );
        }
        if let Some(value) = patch.max_bytes {
            table.insert(
                "max_bytes",
                toml_edit::value(signed_integer(value, "deep.max_bytes")?),
            );
        }
        if let Some(value) = patch.max_redirects {
            table.insert(
                "max_redirects",
                toml_edit::value(signed_integer(value, "deep.max_redirects")?),
            );
        }
        if let Some(value) = patch.max_crawl_urls {
            table.insert(
                "max_crawl_urls",
                toml_edit::value(signed_integer(value, "deep.max_crawl_urls")?),
            );
        }
        if let Some(value) = patch.max_depth {
            table.insert(
                "max_depth",
                toml_edit::value(signed_integer(value, "deep.max_depth")?),
            );
        }
        if let Some(value) = patch.respect_robots {
            table.insert("respect_robots", toml_edit::value(value));
        }
        if let Some(value) = patch.robots_timeout_ms {
            table.insert(
                "robots_timeout_ms",
                toml_edit::value(signed_integer(value, "deep.robots_timeout_ms")?),
            );
        }
        if let Some(value) = patch.robots_max_bytes {
            table.insert(
                "robots_max_bytes",
                toml_edit::value(signed_integer(value, "deep.robots_max_bytes")?),
            );
        }
        if let Some(value) = patch.timeout_ms {
            table.insert(
                "timeout_ms",
                toml_edit::value(signed_integer(value, "deep.timeout_ms")?),
            );
        }
        std::fs::write(path, document.to_string())?;
        Ok(())
    }

    /// Write every field `patch` sets to `[deep.extractor]`, and nothing
    /// else. Same guarantees as [`Config::set_inference_fields`].
    pub fn set_deep_extractor_fields(
        path: &Path,
        patch: &ExtractorConfigPatch,
    ) -> Result<(), ConfigError> {
        let text = std::fs::read_to_string(path)?;
        let mut document = text
            .parse::<toml_edit::DocumentMut>()
            .map_err(|error| ConfigError::Policy(format!("invalid TOML: {error}")))?;
        let table = Self::table_at(&mut document, &["deep", "extractor"])?;
        if let Some(value) = &patch.executable {
            table.insert("executable", toml_edit::value(value.as_str()));
        }
        if let Some(value) = &patch.version {
            table.insert("version", toml_edit::value(value.as_str()));
        }
        if let Some(value) = patch.timeout_ms {
            table.insert(
                "timeout_ms",
                toml_edit::value(signed_integer(value, "deep.extractor.timeout_ms")?),
            );
        }
        if let Some(value) = patch.max_output_bytes {
            table.insert(
                "max_output_bytes",
                toml_edit::value(signed_integer(value, "deep.extractor.max_output_bytes")?),
            );
        }
        std::fs::write(path, document.to_string())?;
        Ok(())
    }

    /// Write every field `patch` sets to `[deep.renderer]`, and nothing
    /// else. Same guarantees as [`Config::set_inference_fields`]; see
    /// [`RendererConfigPatch`] for the `isolated`-profile interaction the
    /// caller's pre-write validation is expected to catch.
    pub fn set_deep_renderer_fields(
        path: &Path,
        patch: &RendererConfigPatch,
    ) -> Result<(), ConfigError> {
        let text = std::fs::read_to_string(path)?;
        let mut document = text
            .parse::<toml_edit::DocumentMut>()
            .map_err(|error| ConfigError::Policy(format!("invalid TOML: {error}")))?;
        let table = Self::table_at(&mut document, &["deep", "renderer"])?;
        if let Some(value) = patch.enabled {
            table.insert("enabled", toml_edit::value(value));
        }
        if let Some(value) = patch.max_browser_calls {
            table.insert(
                "max_browser_calls",
                toml_edit::value(signed_integer(value, "deep.renderer.max_browser_calls")?),
            );
        }
        if let Some(value) = patch.timeout_ms {
            table.insert(
                "timeout_ms",
                toml_edit::value(signed_integer(value, "deep.renderer.timeout_ms")?),
            );
        }
        if let Some(value) = patch.shutdown_grace_ms {
            table.insert(
                "shutdown_grace_ms",
                toml_edit::value(signed_integer(value, "deep.renderer.shutdown_grace_ms")?),
            );
        }
        if let Some(value) = patch.max_memory_mb {
            table.insert(
                "max_memory_mb",
                toml_edit::value(signed_integer(value, "deep.renderer.max_memory_mb")?),
            );
        }
        if let Some(value) = patch.max_redirects {
            table.insert(
                "max_redirects",
                toml_edit::value(signed_integer(value, "deep.renderer.max_redirects")?),
            );
        }
        if let Some(value) = &patch.sandbox_path {
            table.insert("sandbox_path", toml_edit::value(value.as_str()));
        }
        if let Some(value) = patch.max_dom_bytes {
            table.insert(
                "max_dom_bytes",
                toml_edit::value(signed_integer(value, "deep.renderer.max_dom_bytes")?),
            );
        }
        std::fs::write(path, document.to_string())?;
        Ok(())
    }

    /// Deep pipeline safety limits: robots retrieval, crawl/fetch ceilings,
    /// the extractor and renderer bounds, and the ranking-v2 / gap
    /// sub-policies.
    pub(super) fn validate_deep(&self) -> Result<(), ConfigError> {
        // Robots retrieval limits are bounded unconditionally (see the note on
        // `persistence`): the values are written to the file even while
        // `respect_robots` is off, and must never wrap negative on write.
        if !(100..=30_000).contains(&self.deep.robots_timeout_ms)
            || !(1_024..=1_048_576).contains(&self.deep.robots_max_bytes)
        {
            return Err(ConfigError::Policy(
                "invalid robots.txt retrieval limit".into(),
            ));
        }
        // Deep safety limits, bounded in both directions (see the note on
        // `persistence`). Everything here is written to the file as an i64 and
        // must stay far below `i64::MAX`.
        if !(1..=1_000).contains(&self.deep.top_k)
            || !(1..=1_000).contains(&self.deep.max_fetches)
            || !(1..=4 * 1024 * 1024 * 1024).contains(&self.deep.max_bytes)
            || !(0..=100).contains(&self.deep.max_redirects)
            || !(1..=10_000).contains(&self.deep.max_crawl_urls)
            || self.deep.max_depth > 2
            || !(1..=86_400_000).contains(&self.deep.timeout_ms)
            || !(1..=86_400_000).contains(&self.deep.extractor.timeout_ms)
            || !(1..=4 * 1024 * 1024 * 1024).contains(&self.deep.extractor.max_output_bytes)
            || !(1..=1_000_000).contains(&self.deep.renderer.max_browser_calls)
            || !(1..=86_400_000).contains(&self.deep.renderer.timeout_ms)
            || !(1..=86_400_000).contains(&self.deep.renderer.shutdown_grace_ms)
            || !(1..=1_048_576).contains(&self.deep.renderer.max_memory_mb)
            || !(0..=100).contains(&self.deep.renderer.max_redirects)
            || !(1..=4 * 1024 * 1024 * 1024).contains(&self.deep.renderer.max_dom_bytes)
        {
            return Err(ConfigError::Policy("invalid Deep safety limit".into()));
        }
        self.deep
            .ranking_v2
            .policy
            .validate()
            .map_err(|error| ConfigError::Policy(error.to_string()))?;
        self.deep
            .gaps
            .policy
            .validate()
            .map_err(|error| ConfigError::Policy(error.to_string()))?;
        if self.deep.gaps.max_subqueries > 2
            || self.deep.gaps.max_cost == 0
            || self.deep.gaps.max_provider_calls_per_subquery == 0
            || self.deep.gaps.timeout_ms == 0
        {
            return Err(ConfigError::Policy(
                "invalid Gap Analyzer safety limit".into(),
            ));
        }
        Ok(())
    }
}
