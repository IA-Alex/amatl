//! STEP 4D — real pure-Rust Candle embedding backend, in-workspace.
//!
//! Adapted from the STEP 4B reference implementation
//! (`experiments/embedding-relevance/src/candle_backend.rs`). Refactored for
//! clean workspace integration:
//!
//! * loads from an already-validated [`ModelPackage`] — **no download path**,
//!   no `curl`, no HF hub. A missing/corrupt package is caught upstream in
//!   [`model_config`](super::model_config) and never reaches here.
//! * implements the workspace seam's [`EmbeddingBackend`] trait: fallible with
//!   [`EmbeddingUnavailable`], **never panics**, never blocks on I/O after
//!   construction.
//! * exposes true batch document embedding ([`Self::embed_documents`]) so the
//!   optimized pipeline embeds a whole candidate set in one forward pass.
//!
//! Model: `BAAI/bge-small-en-v1.5` (BERT, 384-dim, MIT). Retrieval convention:
//! the query gets the instruction prefix, documents are raw; CLS pooling then
//! L2 normalization.

use std::sync::Mutex;

use candle_core::{DType, Device, IndexOp, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::bert::{BertModel, Config};
use tokenizers::Tokenizer;

use super::model_config::{ModelPackage, ModelPackageError};
use super::{EmbeddingBackend, EmbeddingUnavailable};

const QUERY_PREFIX: &str = "Represent this sentence for searching relevant passages: ";
/// bge-small-en-v1.5 was trained with a 512-token limit.
const MAX_TOKENS: usize = 512;

/// A loaded pure-Rust `bge-small-en-v1.5` on CPU.
pub struct CandleBackend {
    model: BertModel,
    tokenizer: Tokenizer,
    device: Device,
    /// `BertModel::forward` takes `&self`; the `Mutex` keeps the trait object
    /// `Send + Sync` and serializes forward passes (cheap on CPU).
    lock: Mutex<()>,
    dim: usize,
}

impl CandleBackend {
    /// Load the model described by an already-validated [`ModelPackage`].
    ///
    /// Every failure is a typed [`ModelPackageError`] — never a panic. This
    /// does real work (mmap + parse ~128 MB of weights) and is intended to be
    /// called **once**, lazily, and cached.
    pub fn load(pkg: &ModelPackage) -> Result<Self, ModelPackageError> {
        let device = Device::Cpu;

        let config_bytes = std::fs::read(pkg.config_path())
            .map_err(|e| ModelPackageError::Unreadable(format!("config.json: {e}")))?;
        let config: Config = serde_json::from_slice(&config_bytes)
            .map_err(|e| ModelPackageError::Unreadable(format!("config.json parse: {e}")))?;

        let mut tokenizer = Tokenizer::from_file(pkg.tokenizer_path())
            .map_err(|e| ModelPackageError::Unreadable(format!("tokenizer.json: {e}")))?;
        let _ = tokenizer.with_truncation(Some(tokenizers::TruncationParams {
            max_length: MAX_TOKENS,
            ..Default::default()
        }));
        tokenizer.with_padding(Some(tokenizers::PaddingParams {
            strategy: tokenizers::PaddingStrategy::BatchLongest,
            ..Default::default()
        }));

        let weights_path = pkg.weights_path();
        // SAFETY of the underlying mmap is upheld by candle; `unsafe_code` is
        // forbidden in this crate, so use the safe buffered loader instead of
        // `from_mmaped_safetensors`.
        let weights = std::fs::read(&weights_path)
            .map_err(|e| ModelPackageError::Unreadable(format!("model.safetensors: {e}")))?;
        let vb = VarBuilder::from_buffered_safetensors(weights, DType::F32, &device)
            .map_err(|e| ModelPackageError::Unreadable(format!("safetensors: {e}")))?;
        let model = BertModel::load(vb, &config)
            .map_err(|e| ModelPackageError::Unreadable(format!("bert weights: {e}")))?;
        let dim = config.hidden_size;

        Ok(Self {
            model,
            tokenizer,
            device,
            lock: Mutex::new(()),
            dim,
        })
    }

    /// Encode a rectangular batch. Empty strings must be filtered by the caller.
    fn encode_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbeddingUnavailable> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let _g = self
            .lock
            .lock()
            .map_err(|_| EmbeddingUnavailable::BackendError("mutex poisoned".into()))?;

        let encodings = self
            .tokenizer
            .encode_batch(texts.to_vec(), true)
            .map_err(|e| EmbeddingUnavailable::BackendError(format!("tokenize: {e}")))?;

        let bsz = encodings.len();
        let seq = encodings[0].get_ids().len();
        let mut ids: Vec<i64> = Vec::with_capacity(bsz * seq);
        let mut mask: Vec<u32> = Vec::with_capacity(bsz * seq);
        for enc in &encodings {
            ids.extend(enc.get_ids().iter().map(|&x| i64::from(x)));
            mask.extend(enc.get_attention_mask().iter().copied());
        }

        let run = || -> candle_core::Result<Vec<Vec<f32>>> {
            let token_ids = Tensor::from_vec(ids, (bsz, seq), &self.device)?;
            let token_type_ids = token_ids.zeros_like()?;
            let attention_mask = Tensor::from_vec(mask, (bsz, seq), &self.device)?;
            let sequence_output =
                self.model
                    .forward(&token_ids, &token_type_ids, Some(&attention_mask))?;
            let cls = sequence_output.i((.., 0, ..))?;
            let norm = cls.sqr()?.sum_keepdim(1)?.sqrt()?;
            let normalized = cls.broadcast_div(&norm)?;
            normalized.to_vec2()
        };

        run().map_err(|e| EmbeddingUnavailable::BackendError(format!("bert forward: {e}")))
    }
}

impl EmbeddingBackend for CandleBackend {
    fn dimension(&self) -> usize {
        self.dim
    }

    fn embed_query(&self, text: &str) -> Result<Vec<f32>, EmbeddingUnavailable> {
        if text.trim().is_empty() {
            return Err(EmbeddingUnavailable::EmptyInput);
        }
        let prefixed = format!("{QUERY_PREFIX}{text}");
        self.encode_batch(std::slice::from_ref(&prefixed))?
            .into_iter()
            .next()
            .ok_or_else(|| EmbeddingUnavailable::BackendError("no query embedding".into()))
    }

    fn embed_document(&self, text: &str) -> Result<Vec<f32>, EmbeddingUnavailable> {
        if text.trim().is_empty() {
            return Err(EmbeddingUnavailable::EmptyInput);
        }
        self.encode_batch(std::slice::from_ref(&text.to_string()))?
            .into_iter()
            .next()
            .ok_or_else(|| EmbeddingUnavailable::BackendError("no document embedding".into()))
    }
}

impl CandleBackend {
    /// True batched document embedding: one BERT forward pass for the whole
    /// candidate set. Empty inputs yield a zero vector in that slot (cosine 0),
    /// never an error for the batch.
    pub fn embed_documents(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbeddingUnavailable> {
        let mut out = vec![vec![0.0f32; self.dim.max(1)]; texts.len()];
        let mut idx = Vec::new();
        let mut batch = Vec::new();
        for (i, t) in texts.iter().enumerate() {
            if t.trim().is_empty() {
                continue;
            }
            idx.push(i);
            batch.push(t.clone());
        }
        if !batch.is_empty() {
            for (slot, emb) in idx.into_iter().zip(self.encode_batch(&batch)?) {
                out[slot] = emb;
            }
        }
        Ok(out)
    }
}
