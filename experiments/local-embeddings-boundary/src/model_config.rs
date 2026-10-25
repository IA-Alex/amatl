//! STEP 4D — optional model package: paths, pinned hashes, offline validation.
//!
//! Preserves the STEP 4C decision `MODEL_DISTRIBUTION_OPTIONAL_PACKAGE`: the
//! ~128 MB `bge-small-en-v1.5` weights are **never vendored into the core
//! binary**. They ship as a separate, optional package that an operator drops
//! next to the binary (or points at with `MODEL_PATH`). If the package is
//! absent, unreadable, or fails its pinned-hash check, the experimental seam
//! degrades cleanly to the bounded-semantic outcome — search stays fully
//! usable.
//!
//! ## Hard rules
//!
//! * **No network.** This module only ever `stat`s / reads local files. There
//!   is no download path here — a missing file is
//!   [`ModelPackageError::ModelMissing`], never a fetch.
//! * **Fail closed.** Any hash mismatch or corrupt file is an error, not a
//!   best-effort load.
//! * **Explicit config.** `MODEL_PATH`, `MODEL_VERSION`, `MODEL_HASH`,
//!   `TOKENIZER_HASH`, `CONFIG_HASH` are all caller-provided (env or config
//!   file). Nothing is inferred from the network.

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

/// File names inside a model package directory. Fixed — the package layout is
/// part of the contract.
pub const WEIGHTS_FILE: &str = "model.safetensors";
pub const TOKENIZER_FILE: &str = "tokenizer.json";
pub const CONFIG_FILE: &str = "config.json";

/// The model this experimental build was pinned against (STEP 4B / 4C).
pub const PINNED_MODEL_VERSION: &str = "BAAI/bge-small-en-v1.5";

/// Pinned sha256 digests of the three package files for
/// [`PINNED_MODEL_VERSION`]. Measured from the exact files fetched during
/// STEP 4D integration (`huggingface.co/BAAI/bge-small-en-v1.5/resolve/main`).
/// A package whose files do not match these is rejected.
pub const PINNED_HASHES: ModelHashes = ModelHashes {
    model_version: PINNED_MODEL_VERSION,
    weights_sha256: "3c9f31665447c8911517620762200d2245a2518d6e7208acc78cd9db317e21ad",
    tokenizer_sha256: "d241a60d5e8f04cc1b2b3e9ef7a4921b27bf526d9f6050ab90f9267a1f9e5c66",
    config_sha256: "094f8e891b932f2000c92cfc663bac4c62069f5d8af5b5278c4306aef3084750",
};

/// Expected identity of a model package: a version string and the sha256 of
/// each of its three files.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelHashes {
    pub model_version: &'static str,
    pub weights_sha256: &'static str,
    pub tokenizer_sha256: &'static str,
    pub config_sha256: &'static str,
}

/// Why a model package could not be used. Every variant maps to a clean
/// bounded-semantic fallback in [`super::pipeline`]; none is ever surfaced to a
/// search caller or turned into a panic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelPackageError {
    /// No package directory / weights file at the configured path.
    ModelMissing,
    /// Tokenizer file absent.
    TokenizerMissing,
    /// Config file absent.
    ConfigMissing,
    /// A file is present but could not be read.
    Unreadable(String),
    /// `MODEL_VERSION` did not match the version this build is pinned to.
    VersionMismatch { expected: String, got: String },
    /// A file's sha256 did not match the pinned digest.
    HashMismatch { file: &'static str },
}

impl std::fmt::Display for ModelPackageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ModelMissing => write!(f, "model package missing"),
            Self::TokenizerMissing => write!(f, "model package tokenizer missing"),
            Self::ConfigMissing => write!(f, "model package config missing"),
            Self::Unreadable(p) => write!(f, "model package file unreadable: {p}"),
            Self::VersionMismatch { expected, got } => {
                write!(f, "model version mismatch: expected {expected}, got {got}")
            }
            Self::HashMismatch { file } => write!(f, "model package hash mismatch: {file}"),
        }
    }
}

impl std::error::Error for ModelPackageError {}

/// A validated, on-disk model package. Holding one of these is proof that the
/// three files exist and match [`PINNED_HASHES`]; it does **not** mean the
/// model has been loaded into memory yet (that is [`super::CandleBackend`]).
#[derive(Debug, Clone)]
pub struct ModelPackage {
    dir: PathBuf,
    version: String,
}

impl ModelPackage {
    /// Directory the package lives in.
    pub fn dir(&self) -> &Path {
        &self.dir
    }
    pub fn weights_path(&self) -> PathBuf {
        self.dir.join(WEIGHTS_FILE)
    }
    pub fn tokenizer_path(&self) -> PathBuf {
        self.dir.join(TOKENIZER_FILE)
    }
    pub fn config_path(&self) -> PathBuf {
        self.dir.join(CONFIG_FILE)
    }
    pub fn version(&self) -> &str {
        &self.version
    }

    /// Resolve and validate a model package rooted at `dir`.
    ///
    /// `declared_version` is the caller's `MODEL_VERSION` (env / config). When
    /// `Some`, it must equal [`PINNED_MODEL_VERSION`]; when `None` the check is
    /// skipped (the hash check still fully pins identity).
    ///
    /// Purely local: `stat` + `read`. No network under any circumstance.
    pub fn resolve(
        dir: impl AsRef<Path>,
        declared_version: Option<&str>,
    ) -> Result<Self, ModelPackageError> {
        Self::resolve_with(dir, declared_version, &PINNED_HASHES)
    }

    /// [`Self::resolve`] with an explicit expected-hash set (for tests).
    pub fn resolve_with(
        dir: impl AsRef<Path>,
        declared_version: Option<&str>,
        expected: &ModelHashes,
    ) -> Result<Self, ModelPackageError> {
        let dir = dir.as_ref().to_path_buf();

        if let Some(v) = declared_version {
            if v != expected.model_version {
                return Err(ModelPackageError::VersionMismatch {
                    expected: expected.model_version.to_string(),
                    got: v.to_string(),
                });
            }
        }

        let weights = dir.join(WEIGHTS_FILE);
        let tokenizer = dir.join(TOKENIZER_FILE);
        let config = dir.join(CONFIG_FILE);

        if !weights.is_file() {
            return Err(ModelPackageError::ModelMissing);
        }
        if !tokenizer.is_file() {
            return Err(ModelPackageError::TokenizerMissing);
        }
        if !config.is_file() {
            return Err(ModelPackageError::ConfigMissing);
        }

        verify_hash(&config, expected.config_sha256, CONFIG_FILE)?;
        verify_hash(&tokenizer, expected.tokenizer_sha256, TOKENIZER_FILE)?;
        verify_hash(&weights, expected.weights_sha256, WEIGHTS_FILE)?;

        Ok(Self {
            dir,
            version: expected.model_version.to_string(),
        })
    }
}

fn verify_hash(
    path: &Path,
    expected_hex: &str,
    label: &'static str,
) -> Result<(), ModelPackageError> {
    let mut file = std::fs::File::open(path)
        .map_err(|e| ModelPackageError::Unreadable(format!("{}: {e}", path.display())))?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher)
        .map_err(|e| ModelPackageError::Unreadable(format!("{}: {e}", path.display())))?;
    let got = hex_lower(&hasher.finalize());
    if got != expected_hex {
        return Err(ModelPackageError::HashMismatch { file: label });
    }
    Ok(())
}

fn hex_lower(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(char::from_digit((b >> 4) as u32, 16).unwrap());
        s.push(char::from_digit((b & 0xf) as u32, 16).unwrap());
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write_pkg(dir: &Path, weights: &[u8], tok: &[u8], cfg: &[u8]) {
        fs::create_dir_all(dir).unwrap();
        fs::write(dir.join(WEIGHTS_FILE), weights).unwrap();
        fs::write(dir.join(TOKENIZER_FILE), tok).unwrap();
        fs::write(dir.join(CONFIG_FILE), cfg).unwrap();
    }

    fn hashes_for(weights: &[u8], tok: &[u8], cfg: &[u8]) -> ModelHashes {
        // Leak the hex strings so we get 'static — test-only.
        let h = |b: &[u8]| -> &'static str {
            let mut hasher = Sha256::new();
            hasher.update(b);
            Box::leak(hex_lower(&hasher.finalize()).into_boxed_str())
        };
        ModelHashes {
            model_version: PINNED_MODEL_VERSION,
            weights_sha256: h(weights),
            tokenizer_sha256: h(tok),
            config_sha256: h(cfg),
        }
    }

    #[test]
    fn valid_package_resolves() {
        let tmp = std::env::temp_dir().join("amatl_mp_ok");
        let _ = fs::remove_dir_all(&tmp);
        write_pkg(&tmp, b"W", b"T", b"C");
        let expected = hashes_for(b"W", b"T", b"C");
        let pkg = ModelPackage::resolve_with(&tmp, Some(PINNED_MODEL_VERSION), &expected).unwrap();
        assert_eq!(pkg.version(), PINNED_MODEL_VERSION);
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn missing_weights_is_model_missing() {
        let tmp = std::env::temp_dir().join("amatl_mp_missing");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();
        assert_eq!(
            ModelPackage::resolve(&tmp, None).unwrap_err(),
            ModelPackageError::ModelMissing
        );
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn missing_tokenizer_and_config_detected() {
        let tmp = std::env::temp_dir().join("amatl_mp_partial");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();
        fs::write(tmp.join(WEIGHTS_FILE), b"W").unwrap();
        assert_eq!(
            ModelPackage::resolve(&tmp, None).unwrap_err(),
            ModelPackageError::TokenizerMissing
        );
        fs::write(tmp.join(TOKENIZER_FILE), b"T").unwrap();
        assert_eq!(
            ModelPackage::resolve(&tmp, None).unwrap_err(),
            ModelPackageError::ConfigMissing
        );
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn hash_mismatch_fails_closed() {
        let tmp = std::env::temp_dir().join("amatl_mp_corrupt");
        let _ = fs::remove_dir_all(&tmp);
        write_pkg(&tmp, b"W-corrupt", b"T", b"C");
        let expected = hashes_for(b"W", b"T", b"C");
        assert_eq!(
            ModelPackage::resolve_with(&tmp, None, &expected).unwrap_err(),
            ModelPackageError::HashMismatch { file: WEIGHTS_FILE }
        );
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn version_mismatch_rejected() {
        let tmp = std::env::temp_dir().join("amatl_mp_ver");
        let _ = fs::remove_dir_all(&tmp);
        write_pkg(&tmp, b"W", b"T", b"C");
        let expected = hashes_for(b"W", b"T", b"C");
        let err =
            ModelPackage::resolve_with(&tmp, Some("some/other-model"), &expected).unwrap_err();
        assert!(matches!(err, ModelPackageError::VersionMismatch { .. }));
        let _ = fs::remove_dir_all(&tmp);
    }
}
