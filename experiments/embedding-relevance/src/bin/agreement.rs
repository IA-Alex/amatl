//! STEP 4B §3 — ONNX (fastembed) vs Candle numerical/semantic agreement on a
//! fixed diagnostic set. Reports per-pair cosine agreement of the two
//! backends' embeddings and Spearman rank correlation of query/document
//! similarity scores. Does NOT assume numerical identity.
//!
//! Usage:
//!   cargo run --release --bin agreement

use amatl_embedding_relevance_experiment::candle_backend::CandleBackend;
use amatl_embedding_relevance_experiment::corpus;
use amatl_embedding_relevance_experiment::fastembed_backend::FastembedBackend;
use amatl_embedding_relevance_experiment::{cosine_similarity, EmbeddingBackend};

fn spearman(a: &[f64], b: &[f64]) -> f64 {
    let rank = |v: &[f64]| -> Vec<f64> {
        let mut idx: Vec<usize> = (0..v.len()).collect();
        idx.sort_by(|&i, &j| v[i].partial_cmp(&v[j]).unwrap());
        let mut r = vec![0.0; v.len()];
        let mut i = 0;
        while i < idx.len() {
            let mut j = i;
            while j + 1 < idx.len() && (v[idx[j + 1]] - v[idx[i]]).abs() < 1e-12 {
                j += 1;
            }
            let avg = (i + j) as f64 / 2.0 + 1.0;
            for k in i..=j {
                r[idx[k]] = avg;
            }
            i = j + 1;
        }
        r
    };
    let ra = rank(a);
    let rb = rank(b);
    let n = a.len() as f64;
    let mean = n.mul_add(0.0, (n + 1.0) / 2.0);
    let (mut cov, mut va, mut vb) = (0.0, 0.0, 0.0);
    for i in 0..a.len() {
        let da = ra[i] - mean;
        let db = rb[i] - mean;
        cov += da * db;
        va += da * da;
        vb += db * db;
    }
    if va == 0.0 || vb == 0.0 {
        return f64::NAN;
    }
    cov / (va.sqrt() * vb.sqrt())
}

fn main() -> anyhow::Result<()> {
    let onnx = FastembedBackend::bge_small_en_v15()?;
    let cand = CandleBackend::bge_small_en_v15()?;
    println!("ONNX  = {} (dim {})", onnx.id(), onnx.dim());
    println!("CANDLE = {} (dim {})\n", cand.id(), cand.dim());

    let mut rows = Vec::new();
    for path in ["corpus.json", "holdout.json"] {
        let cor = corpus::load(corpus::fixtures_dir().join(path))?;
        rows.extend(cor.samples);
    }
    println!("diagnostic set: {} query/document pairs\n", rows.len());

    // Per-vector agreement: cosine( onnx(x), candle(x) ) for queries and docs.
    let mut vec_agree = Vec::new();
    // Per-pair similarity scores, each backend.
    let mut sim_onnx = Vec::new();
    let mut sim_cand = Vec::new();

    for s in &rows {
        let q = &s.query;
        let d = s.document_text();
        if d.trim().is_empty() {
            continue;
        }

        let q_onnx = onnx.embed_query(q)?;
        let q_cand = cand.embed_query(q)?;
        let d_onnx = onnx.embed_document(&d)?;
        let d_cand = cand.embed_document(&d)?;

        vec_agree.push(cosine_similarity(&q_onnx, &q_cand) as f64);
        vec_agree.push(cosine_similarity(&d_onnx, &d_cand) as f64);

        sim_onnx.push(cosine_similarity(&q_onnx, &d_onnx) as f64);
        sim_cand.push(cosine_similarity(&q_cand, &d_cand) as f64);
    }

    vec_agree.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let mean = |v: &[f64]| v.iter().sum::<f64>() / v.len() as f64;
    let sim_abs_diff: Vec<f64> = sim_onnx
        .iter()
        .zip(&sim_cand)
        .map(|(a, b)| (a - b).abs())
        .collect();

    println!("# EMBEDDING VECTOR AGREEMENT  (cosine of ONNX vs Candle embedding of the same text)");
    println!(
        "ONNX_CANDLE_COSINE_AGREEMENT_MEAN = {:.4}",
        mean(&vec_agree)
    );
    println!(
        "ONNX_CANDLE_COSINE_AGREEMENT_MIN  = {:.4}",
        vec_agree.first().copied().unwrap_or(f64::NAN)
    );
    println!(
        "ONNX_CANDLE_COSINE_AGREEMENT_P05  = {:.4}",
        vec_agree[vec_agree.len() / 20]
    );

    println!("\n# SIMILARITY-SCORE AGREEMENT  (query~document cosine, per pair)");
    println!("SIM_ABS_DIFF_MEAN = {:.4}", mean(&sim_abs_diff));
    println!(
        "SIM_ABS_DIFF_MAX  = {:.4}",
        sim_abs_diff.iter().cloned().fold(0.0, f64::max)
    );
    println!(
        "ONNX_CANDLE_RANK_CORRELATION = {:.4}",
        spearman(&sim_onnx, &sim_cand)
    );

    Ok(())
}
