use super::*;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Backend limits for the optional inference layer.
///
/// The mode itself lives in `data_policy.inference`; this section only sizes
/// the backend that mode selects. See [`crate::inference`].
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct InferenceConfig {
    /// Backend identifier used by `local_only`.
    pub backend: String,
    /// Width of the embedding vectors.
    pub embedding_dimensions: usize,
    /// Maximum documents scored in one ranking call.
    pub max_documents: usize,
    /// Maximum characters read per document before embedding.
    pub max_input_chars: usize,
    /// Weight the reranker keeps for the upstream relevance signal.
    pub reranker_prior_weight: f64,
    /// Path to a local word-vector model file used by the `local_model_v1`
    /// backend. Each line is `token v0 v1 … vN` (space separated floats).
    /// When unset, `local_model_v1` fails closed and Deep degrades to the
    /// hashing backend.
    pub local_model_path: Option<String>,
    /// Capacity of the on-disk embedding cache of `local_model_v1`, in units of
    /// 64 entries (the stored value is multiplied by 64 in `inference.rs`, and
    /// backs [`crate::inference::EmbeddingCache`]).
    ///
    /// This does **not** cap how many documents are embedded per call — nothing
    /// in the pipeline batches on it. It was called `local_model_batch` through
    /// rc.1; that spelling is still accepted as a serde alias.
    // TODO(1.0): drop the `local_model_batch` alias.
    #[serde(alias = "local_model_batch")]
    pub local_cache_capacity: usize,
    /// Optional path where computed embeddings are cached between executions.
    /// The cache is namespaced by the vector-space identity so artifacts are
    /// never reused across backends or widths. When unset, no cache is used.
    pub local_cache_path: Option<String>,
    /// Embeddings endpoint used by `data_policy.inference = "remote_explicit"`.
    /// Must be absolute HTTPS, or HTTP on loopback for a self-hosted server,
    /// and must not embed credentials.
    pub remote_endpoint: Option<String>,
    /// Model identifier sent in the remote request body.
    pub remote_model: Option<String>,
    /// Environment variable holding the bearer credential, when the endpoint
    /// requires one. The value is never written to configuration or logs.
    pub remote_credential_env: Option<String>,
    /// Deadline for one remote embeddings request.
    pub remote_timeout_ms: u64,
    /// Largest number of inputs sent in a single remote request.
    pub remote_max_batch: usize,
}

impl Default for InferenceConfig {
    fn default() -> Self {
        Self {
            backend: crate::inference::LOCAL_EMBEDDING_BACKEND_ID.into(),
            embedding_dimensions: 256,
            max_documents: 64,
            max_input_chars: 20_000,
            reranker_prior_weight: 0.5,
            local_model_path: None,
            local_cache_capacity: 32,
            local_cache_path: None,
            remote_endpoint: None,
            remote_model: None,
            remote_credential_env: None,
            remote_timeout_ms: 5_000,
            remote_max_batch: 32,
        }
    }
}

/// Every [`InferenceConfig`] field an admin-scoped caller may update in one
/// request, each optional so only the fields actually sent are changed.
///
/// On the `Option<String>` fields that are themselves optional in
/// [`InferenceConfig`] (`local_model_path`, `local_cache_path`,
/// `remote_endpoint`, `remote_model`, `remote_credential_env`): submitting an
/// empty string clears the field (sets it back to `None`); omitting the
/// field from the request leaves it exactly as it was. There is no separate
/// "clear" flag — an admin who wants to blank a value out sends `""`.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct InferenceConfigPatch {
    pub backend: Option<String>,
    pub embedding_dimensions: Option<usize>,
    pub max_documents: Option<usize>,
    pub max_input_chars: Option<usize>,
    pub reranker_prior_weight: Option<f64>,
    pub local_model_path: Option<String>,
    // TODO(1.0): drop the `local_model_batch` alias.
    #[serde(alias = "local_model_batch")]
    pub local_cache_capacity: Option<usize>,
    pub local_cache_path: Option<String>,
    pub remote_endpoint: Option<String>,
    pub remote_model: Option<String>,
    pub remote_credential_env: Option<String>,
    pub remote_timeout_ms: Option<u64>,
    pub remote_max_batch: Option<usize>,
}

impl InferenceConfigPatch {
    /// Apply every field this patch sets to `config`, leaving the rest
    /// untouched.
    ///
    /// Mirrors exactly what [`Config::set_inference_fields`] writes to disk
    /// (same "empty string clears" convention on the optional string
    /// fields), so a caller that builds a candidate with this before
    /// validating and writing through that function never validates
    /// something different from what ends up on disk.
    pub fn apply(&self, config: &mut InferenceConfig) {
        if let Some(backend) = &self.backend {
            config.backend = backend.clone();
        }
        if let Some(value) = self.embedding_dimensions {
            config.embedding_dimensions = value;
        }
        if let Some(value) = self.max_documents {
            config.max_documents = value;
        }
        if let Some(value) = self.max_input_chars {
            config.max_input_chars = value;
        }
        if let Some(value) = self.reranker_prior_weight {
            config.reranker_prior_weight = value;
        }
        if let Some(value) = &self.local_model_path {
            config.local_model_path = clearable(value);
        }
        if let Some(value) = self.local_cache_capacity {
            config.local_cache_capacity = value;
        }
        if let Some(value) = &self.local_cache_path {
            config.local_cache_path = clearable(value);
        }
        if let Some(value) = &self.remote_endpoint {
            config.remote_endpoint = clearable(value);
        }
        if let Some(value) = &self.remote_model {
            config.remote_model = clearable(value);
        }
        if let Some(value) = &self.remote_credential_env {
            config.remote_credential_env = clearable(value);
        }
        if let Some(value) = self.remote_timeout_ms {
            config.remote_timeout_ms = value;
        }
        if let Some(value) = self.remote_max_batch {
            config.remote_max_batch = value;
        }
    }
}
/// How the router assigns provider roles.
///
/// `Legacy` preserves the pre-STEP-1 behavior exactly: providers are ordered
/// purely by adaptive score and the first round is filled to
/// `search_policy.first_round_min_providers`. `PrimaryExpansion` makes the
/// role dominate the score — the primary provider always runs alone in round
/// one when it is eligible, and expansion providers are only reached in later
/// rounds through the existing progressive-round decision. Rollback is a
/// one-line change back to `Legacy`.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExpansionMode {
    #[default]
    Legacy,
    PrimaryExpansion,
}

impl ExpansionMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Legacy => "legacy",
            Self::PrimaryExpansion => "primary_expansion",
        }
    }
}

/// Primary / expansion provider roles.
///
/// The declarative default (`primary_provider = "marginalia"`) is always
/// present so an operator only flips `mode` to activate it, but while
/// `mode = "legacy"` nothing in routing consults this section.
/// `expansion_providers` ships **empty**: the expansion role needs a second
/// source, and Marginalia is the only source AMATL ships, so no expansion
/// source exists to name by default. It stays a list, not a single name, so an
/// operator can declare a second source and register it there without a schema
/// change.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ExpansionConfig {
    pub mode: ExpansionMode,
    pub primary_provider: String,
    pub expansion_providers: Vec<String>,
}

impl Default for ExpansionConfig {
    fn default() -> Self {
        Self {
            mode: ExpansionMode::Legacy,
            primary_provider: "marginalia".into(),
            expansion_providers: Vec::new(),
        }
    }
}

impl ExpansionConfig {
    /// Whether the primary/expansion role model is active.
    pub fn role_model_active(&self) -> bool {
        self.mode == ExpansionMode::PrimaryExpansion
    }
}

impl Config {
    /// Write every field `patch` sets to `[inference]`, and nothing else.
    ///
    /// Same guarantees as `Config::set_scalar_field` (comments survive,
    /// caller validates a candidate first) — see
    /// [`InferenceConfigPatch::apply`], which every field here mirrors.
    pub fn set_inference_fields(
        path: &Path,
        patch: &InferenceConfigPatch,
    ) -> Result<(), ConfigError> {
        let text = std::fs::read_to_string(path)?;
        let mut document = text
            .parse::<toml_edit::DocumentMut>()
            .map_err(|error| ConfigError::Policy(format!("invalid TOML: {error}")))?;
        if document.get("inference").is_none() {
            document["inference"] = toml_edit::Item::Table(toml_edit::Table::new());
        }
        let table = document["inference"]
            .as_table_mut()
            .ok_or_else(|| ConfigError::Policy("inference is not a table".into()))?;
        if let Some(backend) = &patch.backend {
            table.insert("backend", toml_edit::value(backend.as_str()));
        }
        if let Some(value) = patch.embedding_dimensions {
            table.insert(
                "embedding_dimensions",
                toml_edit::value(signed_integer(value, "inference.embedding_dimensions")?),
            );
        }
        if let Some(value) = patch.max_documents {
            table.insert(
                "max_documents",
                toml_edit::value(signed_integer(value, "inference.max_documents")?),
            );
        }
        if let Some(value) = patch.max_input_chars {
            table.insert(
                "max_input_chars",
                toml_edit::value(signed_integer(value, "inference.max_input_chars")?),
            );
        }
        if let Some(value) = patch.reranker_prior_weight {
            table.insert("reranker_prior_weight", toml_edit::value(value));
        }
        set_clearable_string(table, "local_model_path", patch.local_model_path.as_deref());
        if let Some(value) = patch.local_cache_capacity {
            table.insert(
                "local_cache_capacity",
                toml_edit::value(signed_integer(value, "inference.local_cache_capacity")?),
            );
        }
        set_clearable_string(table, "local_cache_path", patch.local_cache_path.as_deref());
        set_clearable_string(table, "remote_endpoint", patch.remote_endpoint.as_deref());
        set_clearable_string(table, "remote_model", patch.remote_model.as_deref());
        set_clearable_string(
            table,
            "remote_credential_env",
            patch.remote_credential_env.as_deref(),
        );
        if let Some(value) = patch.remote_timeout_ms {
            table.insert(
                "remote_timeout_ms",
                toml_edit::value(signed_integer(value, "inference.remote_timeout_ms")?),
            );
        }
        if let Some(value) = patch.remote_max_batch {
            table.insert(
                "remote_max_batch",
                toml_edit::value(signed_integer(value, "inference.remote_max_batch")?),
            );
        }
        write_config_document(path, &document)?;
        Ok(())
    }

    /// Replace `[expansion]` wholesale. The caller must have validated a
    /// candidate config with this change applied first, exactly as with
    /// [`Config::set_search_policy`] — `validate_expansion` cross-checks the
    /// role names against the declared providers.
    pub fn set_expansion(path: &Path, expansion: &ExpansionConfig) -> Result<(), ConfigError> {
        Self::replace_table(path, &["expansion"], expansion)
    }
    pub(super) fn validate_inference(&self) -> Result<(), ConfigError> {
        // `backend` selects the *local* embedder. The remote backend is chosen
        // by `data_policy.inference = "remote_explicit"`, never by this key, so
        // naming it here stays an error on purpose.
        match self.inference.backend.as_str() {
            crate::inference::LOCAL_EMBEDDING_BACKEND_ID => {}
            crate::inference::LOCAL_MODEL_BACKEND_ID => {
                let path = self
                    .inference
                    .local_model_path
                    .as_deref()
                    .filter(|value| !value.trim().is_empty())
                    .ok_or_else(|| {
                        ConfigError::Policy(format!(
                            "inference.local_model_path is required with backend = {}",
                            crate::inference::LOCAL_MODEL_BACKEND_ID
                        ))
                    })?;
                if !std::path::Path::new(path).is_absolute() {
                    return Err(ConfigError::Policy(
                        "inference.local_model_path must be an absolute path".into(),
                    ));
                }
            }
            other => {
                return Err(ConfigError::Policy(format!(
                    "unknown inference backend: {other} (expected {} or {}; the remote backend is selected by data_policy.inference)",
                    crate::inference::LOCAL_EMBEDDING_BACKEND_ID,
                    crate::inference::LOCAL_MODEL_BACKEND_ID
                )));
            }
        }
        if self.data_policy.inference == InferenceMode::RemoteExplicit {
            if !self.data_policy.allows_remote_inference() {
                return Err(ConfigError::Policy(
                    "remote_explicit inference requires a standard profile with governed egress"
                        .into(),
                ));
            }
            let endpoint = self.inference.remote_endpoint.as_deref().ok_or_else(|| {
                ConfigError::Policy(
                    "remote_explicit inference requires inference.remote_endpoint".into(),
                )
            })?;
            crate::inference::validate_remote_endpoint(endpoint)
                .map_err(|error| ConfigError::Policy(error.to_string()))?;
            if self
                .inference
                .remote_model
                .as_deref()
                .is_none_or(str::is_empty)
            {
                return Err(ConfigError::Policy(
                    "remote_explicit inference requires inference.remote_model".into(),
                ));
            }
        }
        // Every numeric inference limit is bounded unconditionally, even while
        // `data_policy.inference` is `disabled`/`local_only`: the values are
        // still written to the file as an i64, and an unbounded one would wrap
        // negative on write and poison the file for the next reload or restart.
        // The ceilings are deliberately generous — far above any real
        // deployment, far below `i64::MAX`.
        if !(crate::inference::MINIMUM_EMBEDDING_DIMENSIONS
            ..=crate::inference::MAXIMUM_EMBEDDING_DIMENSIONS)
            .contains(&self.inference.embedding_dimensions)
            || !(1..=1_000_000).contains(&self.inference.max_documents)
            || !(1..=100_000_000).contains(&self.inference.max_input_chars)
            || !(1..=1_000_000).contains(&self.inference.local_cache_capacity)
            || !(1..=crate::inference::MAXIMUM_REMOTE_BATCH)
                .contains(&self.inference.remote_max_batch)
            || !(100..=60_000).contains(&self.inference.remote_timeout_ms)
            || !self.inference.reranker_prior_weight.is_finite()
            || !(0.0..=1.0).contains(&self.inference.reranker_prior_weight)
        {
            return Err(ConfigError::Policy("invalid inference limit".into()));
        }
        let policy = &self.deep.ranking_v2.policy;
        let needs_backend = self.deep.ranking_v2.enabled
            && (policy.weight_semantic > 0.0 || policy.weight_reranker > 0.0);
        if needs_backend {
            match self.data_policy.inference {
                InferenceMode::Disabled => {
                    return Err(ConfigError::Policy(
                        "semantic or reranker ranking weights require an enabled inference mode"
                            .into(),
                    ))
                }
                // `remote_explicit` is validated above; reaching here means an
                // endpoint, model and bounded limits are all present.
                InferenceMode::RemoteExplicit | InferenceMode::LocalOnly => {}
            }
        }
        Ok(())
    }
    /// Validate the primary/expansion role model.
    ///
    /// The declarative names are checked even in `Legacy` mode so a
    /// misconfigured section is caught before an operator flips `mode`, but
    /// only `PrimaryExpansion` makes them load-bearing. A role name must be a
    /// declared provider (the registry builds it), the primary must not also
    /// appear in the expansion list, and the expansion list must not repeat a
    /// name.
    pub(super) fn validate_expansion(&self) -> Result<(), ConfigError> {
        let primary = self.expansion.primary_provider.trim();
        if primary.is_empty() {
            return Err(ConfigError::Policy(
                "expansion.primary_provider must not be empty".into(),
            ));
        }
        if !self.providers.is_declared(primary) {
            return Err(ConfigError::Policy(format!(
                "expansion.primary_provider '{primary}' is not a declared provider"
            )));
        }
        let mut seen = std::collections::BTreeSet::new();
        for name in &self.expansion.expansion_providers {
            let name = name.trim();
            if name.is_empty() {
                return Err(ConfigError::Policy(
                    "expansion.expansion_providers must not contain an empty name".into(),
                ));
            }
            if !self.providers.is_declared(name) {
                return Err(ConfigError::Policy(format!(
                    "expansion.expansion_providers entry '{name}' is not a declared provider"
                )));
            }
            if name == primary {
                return Err(ConfigError::Policy(format!(
                    "provider '{name}' cannot be both primary and expansion"
                )));
            }
            if !seen.insert(name.to_string()) {
                return Err(ConfigError::Policy(format!(
                    "expansion.expansion_providers repeats '{name}'"
                )));
            }
        }
        if self.expansion.role_model_active() && self.expansion.expansion_providers.is_empty() {
            return Err(ConfigError::Policy(
                "expansion.mode = \"primary_expansion\" requires at least one expansion provider"
                    .into(),
            ));
        }
        Ok(())
    }
}
