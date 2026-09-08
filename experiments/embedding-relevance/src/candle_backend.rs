//! STEP 4B — pure-Rust embedding backend using `candle`.
//!
//! Model: BAAI/bge-small-en-v1.5 (BERT architecture, 384-dim), MIT license.
//! Weights are loaded as `model.safetensors` (fp32) from the Hugging Face hub
//! **once at development time** into the crate-local `./.candle_cache`. No
//! network access at inference time.
//!
//! This is a sibling of [`crate::fastembed_backend::FastembedBackend`], not a
//! replacement. Nothing here is wired into amatl-core relevance, routing,
//! ranking or telemetry.
//!
//! bge-small-en-v1.5 retrieval convention: the *query* gets the instruction
//! prefix
//!   "Represent this sentence for searching relevant passages: "
//! documents are embedded raw. Pooling is CLS-token (first position) followed
//! by L2 normalization — this matches `FlagEmbedding` / `sentence-transformers`
//! defaults for this model and what `fastembed` does internally.

use crate::{Embedding, EmbeddingBackend};
use anyhow::{Context, Result};
use candle_core::{DType, Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::bert::{BertModel, Config};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tokenizers::Tokenizer;

const HF_REPO: &str = "BAAI/bge-small-en-v1.5";
const QUERY_PREFIX: &str = "Represent this sentence for searching relevant passages: ";
/// bge-small-en-v1.5 was trained with a 512-token limit.
const MAX_TOKENS: usize = 512;

pub struct CandleBackend {
    model: BertModel,
    tokenizer: Tokenizer,
    device: Device,
    // BertModel::forward takes &self but is not Sync-safe to call concurrently
    // through a shared ref because of internal cublas handles on GPU; on CPU it
    // is fine, but keep a Mutex so the trait object is Send + Sync and the
    // contract matches FastembedBackend.
    lock: Mutex<()>,
    id: String,
    dim: usize,
}

/// Resolve the three model files, downloading them once into `cache_dir` if
/// absent. Returns (config.json, tokenizer.json, model.safetensors) paths.
///
/// The download is an **explicit, dev-time only** fetch over HTTPS from the
/// Hugging Face hub (`resolve/main`), gated on the files being absent. It is
/// never triggered at inference time (all three files present short-circuits),
/// and there is no runtime auto-refresh. This mirrors the STEP 4A ONNX
/// prototype's one-time model fetch. To pre-provision offline, drop the three
/// files into `cache_dir` by hand.
fn resolve_model_files(cache_dir: &Path) -> Result<(PathBuf, PathBuf, PathBuf)> {
    let want = ["config.json", "tokenizer.json", "model.safetensors"];
    let local: Vec<PathBuf> = want.iter().map(|f| cache_dir.join(f)).collect();
    if local.iter().all(|p| p.exists()) {
        return Ok((local[0].clone(), local[1].clone(), local[2].clone()));
    }

    std::fs::create_dir_all(cache_dir).ok();
    for (f, dst) in want.iter().zip(local.iter()) {
        if dst.exists() {
            continue;
        }
        let url = format!("https://huggingface.co/{HF_REPO}/resolve/main/{f}");
        eprintln!("[candle_backend] dev-time model fetch: {url}");
        let status = std::process::Command::new("curl")
            .args(["-fsSL", "--retry", "3", "-o"])
            .arg(dst)
            .arg(&url)
            .status()
            .with_context(|| format!("spawning curl for {f}"))?;
        anyhow::ensure!(status.success(), "curl failed for {url}");
    }
    Ok((local[0].clone(), local[1].clone(), local[2].clone()))
}

impl CandleBackend {
    /// Crate-local cache directory (`./.candle_cache`).
    pub fn default_cache_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join(".candle_cache")
    }

    /// Load bge-small-en-v1.5 on CPU. One-time download if not cached.
    pub fn bge_small_en_v15() -> Result<Self> {
        Self::load(&Self::default_cache_dir())
    }

    pub fn load(cache_dir: &Path) -> Result<Self> {
        let device = Device::Cpu;
        let (config_path, tokenizer_path, weights_path) = resolve_model_files(cache_dir)?;

        let config: Config =
            serde_json::from_slice(&std::fs::read(&config_path).context("reading config.json")?)
                .context("parsing bert config.json")?;

        let mut tokenizer =
            Tokenizer::from_file(&tokenizer_path).map_err(|e| anyhow::anyhow!("tokenizer: {e}"))?;
        // Fixed truncation/padding so batches are rectangular and inputs are
        // bounded (spec §7 LONG_INPUT).
        let _ = tokenizer.with_truncation(Some(tokenizers::TruncationParams {
            max_length: MAX_TOKENS,
            ..Default::default()
        }));
        tokenizer.with_padding(Some(tokenizers::PaddingParams {
            strategy: tokenizers::PaddingStrategy::BatchLongest,
            ..Default::default()
        }));

        let vb = unsafe {
            VarBuilder::from_mmaped_safetensors(&[weights_path], DType::F32, &device)
                .context("mmap safetensors")?
        };
        let model = BertModel::load(vb, &config).context("loading BertModel weights")?;
        let dim = config.hidden_size;

        Ok(Self {
            model,
            tokenizer,
            device,
            lock: Mutex::new(()),
            id: format!("{HF_REPO} (candle, safetensors)"),
            dim,
        })
    }

    fn encode_batch(&self, texts: &[String]) -> Result<Vec<Embedding>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let _g = self.lock.lock().expect("candle backend mutex poisoned");

        let encodings = self
            .tokenizer
            .encode_batch(texts.to_vec(), true)
            .map_err(|e| anyhow::anyhow!("tokenize batch: {e}"))?;

        let bsz = encodings.len();
        let seq = encodings[0].get_ids().len();
        let mut ids: Vec<i64> = Vec::with_capacity(bsz * seq);
        let mut mask: Vec<u32> = Vec::with_capacity(bsz * seq);
        for enc in &encodings {
            ids.extend(enc.get_ids().iter().map(|&x| i64::from(x)));
            mask.extend(enc.get_attention_mask().iter().copied());
        }

        let token_ids = Tensor::from_vec(ids, (bsz, seq), &self.device)?;
        let token_type_ids = token_ids.zeros_like()?;
        // candle's BertModel takes the raw 0/1 attention mask and extends it to
        // the additive form internally.
        let attention_mask = Tensor::from_vec(mask, (bsz, seq), &self.device)?;

        let sequence_output = self
            .model
            .forward(&token_ids, &token_type_ids, Some(&attention_mask))
            .context("bert forward")?;

        // CLS pooling: take position 0 of every sequence.
        let cls = sequence_output.i((.., 0, ..))?; // (bsz, hidden)

        // L2 normalize each row.
        let norm = cls.sqr()?.sum_keepdim(1)?.sqrt()?;
        let normalized = cls.broadcast_div(&norm)?;

        let rows: Vec<Vec<f32>> = normalized.to_vec2()?;
        Ok(rows)
    }
}

use candle_core::IndexOp;

impl EmbeddingBackend for CandleBackend {
    fn id(&self) -> &str {
        &self.id
    }
    fn dim(&self) -> usize {
        self.dim
    }

    fn embed_query(&self, text: &str) -> Result<Embedding> {
        if text.trim().is_empty() {
            return Ok(vec![0.0; self.dim.max(1)]);
        }
        let prefixed = format!("{QUERY_PREFIX}{text}");
        self.encode_batch(&[prefixed])?
            .into_iter()
            .next()
            .context("candle returned no query embedding")
    }

    fn embed_document(&self, text: &str) -> Result<Embedding> {
        if text.trim().is_empty() {
            return Ok(vec![0.0; self.dim.max(1)]);
        }
        self.encode_batch(&[text.to_string()])?
            .into_iter()
            .next()
            .context("candle returned no document embedding")
    }

    fn embed_documents(&self, texts: &[String]) -> Result<Vec<Embedding>> {
        // Replace empty docs with zero vectors, embed the rest as one batch.
        let mut out = vec![Vec::new(); texts.len()];
        let mut idx = Vec::new();
        let mut batch = Vec::new();
        for (i, t) in texts.iter().enumerate() {
            if t.trim().is_empty() {
                out[i] = vec![0.0; self.dim.max(1)];
            } else {
                idx.push(i);
                batch.push(t.clone());
            }
        }
        if !batch.is_empty() {
            for (slot, emb) in idx.into_iter().zip(self.encode_batch(&batch)?) {
                out[slot] = emb;
            }
        }
        Ok(out)
    }
}
