//! Real embedding backend using `fastembed` (ONNX Runtime via `ort`).
//!
//! Model: BAAI/bge-small-en-v1.5 (the `fastembed` default), MIT license,
//! 384-dim, ~133 MB int32 ONNX weights. Downloaded once at development time
//! into the Hugging Face cache; no network access at inference time.
//!
//! bge-small-en-v1.5 expects an instruction prefix on the *query* side for
//! retrieval:
//!   "Represent this sentence for searching relevant passages: <query>"
//! Documents are embedded without a prefix. `fastembed` applies this
//! automatically for `EmbeddingModel::BGESmallENV15` via query/passage APIs.

use crate::{Embedding, EmbeddingBackend};
use anyhow::Context;
use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};
use std::sync::Mutex;

pub struct FastembedBackend {
    inner: Mutex<TextEmbedding>,
    id: String,
    dim: usize,
}

impl FastembedBackend {
    /// Initialize the default model (bge-small-en-v1.5). Triggers a one-time
    /// download if the model is not already in the HF cache.
    pub fn bge_small_en_v15() -> anyhow::Result<Self> {
        let mut model = TextEmbedding::try_new(
            InitOptions::new(EmbeddingModel::BGESmallENV15).with_show_download_progress(true),
        )
        .context("initializing fastembed bge-small-en-v1.5")?;
        // Probe dimension with a throwaway embedding.
        let probe = model.embed(vec!["dimension probe"], None)?;
        let dim = probe.first().map(|v| v.len()).unwrap_or(0);
        Ok(Self {
            inner: Mutex::new(model),
            id: "BAAI/bge-small-en-v1.5".to_string(),
            dim,
        })
    }

    /// Initialize all-MiniLM-L6-v2 (Apache-2.0, 384-dim, ~90 MB) — the second
    /// candidate for the dev-split model comparison.
    pub fn all_minilm_l6_v2() -> anyhow::Result<Self> {
        let mut model = TextEmbedding::try_new(
            InitOptions::new(EmbeddingModel::AllMiniLML6V2).with_show_download_progress(true),
        )
        .context("initializing fastembed all-MiniLM-L6-v2")?;
        let probe = model.embed(vec!["dimension probe"], None)?;
        let dim = probe.first().map(|v| v.len()).unwrap_or(0);
        Ok(Self {
            inner: Mutex::new(model),
            id: "sentence-transformers/all-MiniLM-L6-v2".to_string(),
            dim,
        })
    }
}

fn l2_normalize(mut v: Vec<f32>) -> Vec<f32> {
    let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > f32::EPSILON {
        for x in &mut v {
            *x /= norm;
        }
    }
    v
}

impl EmbeddingBackend for FastembedBackend {
    fn id(&self) -> &str {
        &self.id
    }
    fn dim(&self) -> usize {
        self.dim
    }

    fn embed_query(&self, text: &str) -> anyhow::Result<Embedding> {
        // Empty input: return a zero vector rather than erroring, so the
        // hybrid caller can treat it as "no semantic evidence".
        if text.trim().is_empty() {
            return Ok(vec![0.0; self.dim.max(1)]);
        }
        let mut guard = self.inner.lock().expect("embedding mutex poisoned");
        let out = guard
            .embed(vec![text], None)
            .context("embed_query inference")?;
        let v = out
            .into_iter()
            .next()
            .context("fastembed returned no query embedding")?;
        Ok(l2_normalize(v))
    }

    fn embed_document(&self, text: &str) -> anyhow::Result<Embedding> {
        if text.trim().is_empty() {
            return Ok(vec![0.0; self.dim.max(1)]);
        }
        let mut guard = self.inner.lock().expect("embedding mutex poisoned");
        let out = guard
            .embed(vec![text], None)
            .context("embed_document inference")?;
        let v = out
            .into_iter()
            .next()
            .context("fastembed returned no document embedding")?;
        Ok(l2_normalize(v))
    }

    fn embed_documents(&self, texts: &[String]) -> anyhow::Result<Vec<Embedding>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let mut guard = self.inner.lock().expect("embedding mutex poisoned");
        let refs: Vec<&str> = texts.iter().map(|s| s.as_str()).collect();
        let out = guard
            .embed(refs, None)
            .context("embed_documents batch inference")?;
        Ok(out.into_iter().map(l2_normalize).collect())
    }
}
