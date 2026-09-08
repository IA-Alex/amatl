//! STEP 4B — real performance measurements for the Candle backend.
//! Same methodology as `bench.rs` (the ONNX/fastembed bench) so numbers are
//! directly comparable. All numbers printed here come from executed inference.
//!
//! Usage:
//!   cargo run --release --bin candle_bench

use amatl_embedding_relevance_experiment::candle_backend::CandleBackend;
use amatl_embedding_relevance_experiment::corpus;
use amatl_embedding_relevance_experiment::{cosine_similarity, EmbeddingBackend};
use std::time::Instant;

fn rss_kb() -> Option<u64> {
    let s = std::fs::read_to_string("/proc/self/status").ok()?;
    for line in s.lines() {
        if let Some(rest) = line.strip_prefix("VmRSS:") {
            return rest.trim().trim_end_matches(" kB").trim().parse().ok();
        }
    }
    None
}

fn pctl(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return f64::NAN;
    }
    let idx = ((p / 100.0) * (sorted.len() as f64 - 1.0)).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

fn main() -> anyhow::Result<()> {
    println!("# ENVIRONMENT");
    println!(
        "OSRELEASE = {}",
        std::fs::read_to_string("/proc/sys/kernel/osrelease")
            .unwrap_or_default()
            .trim()
    );
    println!(
        "NPROC = {}",
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(0)
    );
    let rss_before = rss_kb();
    println!("RSS_BEFORE_KB = {rss_before:?}");

    let load_start = Instant::now();
    let backend = CandleBackend::bge_small_en_v15()?;
    let load_ms = load_start.elapsed().as_secs_f64() * 1000.0;
    let rss_after = rss_kb();

    println!("\n# MODEL");
    println!("MODEL_ID = {}", backend.id());
    println!("EMBEDDING_DIMENSION = {}", backend.dim());
    println!("CANDLE_MODEL_LOAD_TIME_MS = {load_ms:.1}");
    println!("RSS_AFTER_LOAD_KB = {rss_after:?}");
    if let (Some(b), Some(a)) = (rss_before, rss_after) {
        println!(
            "CANDLE_RSS_DELTA_MB = {:.1}",
            (a.saturating_sub(b)) as f64 / 1024.0
        );
    }

    let corpus = corpus::load(corpus::fixtures_dir().join("corpus.json"))?;
    let queries: Vec<String> = corpus.samples.iter().map(|s| s.query.clone()).collect();
    let docs: Vec<String> = corpus.samples.iter().map(|s| s.document_text()).collect();

    // Warm-up.
    let _ = backend.embed_query(&queries[0])?;
    let _ = backend.embed_document(&docs[0])?;

    let mut q_ms = Vec::new();
    for q in &queries {
        let t = Instant::now();
        let _ = backend.embed_query(q)?;
        q_ms.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    q_ms.sort_by(|a, b| a.partial_cmp(b).unwrap());

    let mut d_ms = Vec::new();
    for d in &docs {
        let t = Instant::now();
        let _ = backend.embed_document(d)?;
        d_ms.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    d_ms.sort_by(|a, b| a.partial_cmp(b).unwrap());

    let batch10: Vec<String> = docs[..10.min(docs.len())].to_vec();
    let batch20: Vec<String> = docs[..20.min(docs.len())].to_vec();
    let t = Instant::now();
    let _ = backend.embed_documents(&batch10)?;
    let b10 = t.elapsed().as_secs_f64() * 1000.0;
    let t = Instant::now();
    let _ = backend.embed_documents(&batch20)?;
    let b20 = t.elapsed().as_secs_f64() * 1000.0;

    println!(
        "\n# LATENCY (n={} queries, {} docs)",
        q_ms.len(),
        d_ms.len()
    );
    println!("CANDLE_QUERY_EMBED_P50_MS = {:.3}", pctl(&q_ms, 50.0));
    println!("CANDLE_QUERY_EMBED_P95_MS = {:.3}", pctl(&q_ms, 95.0));
    println!("CANDLE_DOCUMENT_EMBED_P50_MS = {:.3}", pctl(&d_ms, 50.0));
    println!("CANDLE_DOCUMENT_EMBED_P95_MS = {:.3}", pctl(&d_ms, 95.0));
    println!("CANDLE_BATCH_10_MS = {b10:.3}");
    println!("CANDLE_BATCH_20_MS = {b20:.3}");

    let mut e2e = Vec::new();
    for chunk in docs.chunks(10) {
        let t = Instant::now();
        let qv = backend.embed_query(&queries[0])?;
        let dvs = backend.embed_documents(chunk)?;
        for dv in &dvs {
            let _ = cosine_similarity(&qv, dv);
        }
        e2e.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    e2e.sort_by(|a, b| a.partial_cmp(b).unwrap());
    println!("CANDLE_END_TO_END_10_DOCS_P50_MS = {:.3}", pctl(&e2e, 50.0));
    println!("CANDLE_END_TO_END_10_DOCS_P95_MS = {:.3}", pctl(&e2e, 95.0));

    Ok(())
}
