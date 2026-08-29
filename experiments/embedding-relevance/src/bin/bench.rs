//! Real performance measurements for the embedding backend.
//! All numbers printed here come from executed inference on this machine.
//!
//! Usage:
//!   cargo run --release --bin bench            # bge-small-en-v1.5
//!   cargo run --release --bin bench -- minilm  # all-MiniLM-L6-v2

use amatl_embedding_relevance_experiment::corpus::{self};
use amatl_embedding_relevance_experiment::fastembed_backend::FastembedBackend;
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
    let which = std::env::args().nth(1).unwrap_or_default();
    println!("# ENVIRONMENT");
    println!("HOSTNAME_UNAME = {}", uname());
    println!(
        "NPROC = {}",
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(0)
    );
    println!("RSS_BEFORE_KB = {:?}", rss_kb());

    let load_start = Instant::now();
    let backend: FastembedBackend = if which == "minilm" {
        FastembedBackend::all_minilm_l6_v2()?
    } else {
        FastembedBackend::bge_small_en_v15()?
    };
    let load_ms = load_start.elapsed().as_secs_f64() * 1000.0;
    let rss_after = rss_kb();

    println!("\n# MODEL");
    println!("MODEL_ID = {}", backend.id());
    println!("EMBEDDING_DIMENSION = {}", backend.dim());
    println!("MODEL_LOAD_TIME_MS = {load_ms:.1}");
    println!("RSS_AFTER_LOAD_KB = {rss_after:?}");
    if let (Some(a), Some(b)) = (rss_kb(), rss_after) {
        let _ = a;
        println!("RSS_DELTA_MB = {:.1}", (b as f64) / 1024.0);
    }

    // Load real query/doc strings from the consumed diagnostic corpus.
    let corpus = corpus::load(corpus::fixtures_dir().join("corpus.json"))?;
    let queries: Vec<String> = corpus.samples.iter().map(|s| s.query.clone()).collect();
    let docs: Vec<String> = corpus.samples.iter().map(|s| s.document_text()).collect();

    // Warm-up.
    let _ = backend.embed_query(&queries[0])?;
    let _ = backend.embed_document(&docs[0])?;

    // Single-query latency.
    let mut q_ms = Vec::new();
    for q in &queries {
        let t = Instant::now();
        let _ = backend.embed_query(q)?;
        q_ms.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    q_ms.sort_by(|a, b| a.partial_cmp(b).unwrap());

    // Single-document latency.
    let mut d_ms = Vec::new();
    for d in &docs {
        let t = Instant::now();
        let _ = backend.embed_document(d)?;
        d_ms.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    d_ms.sort_by(|a, b| a.partial_cmp(b).unwrap());

    // Batch timings.
    let batch10: Vec<String> = docs[..10.min(docs.len())].to_vec();
    let batch20: Vec<String> = docs[..20.min(docs.len())].to_vec();
    let t = Instant::now();
    let _ = backend.embed_documents(&batch10)?;
    let b10 = t.elapsed().as_secs_f64() * 1000.0;
    let t = Instant::now();
    let _ = backend.embed_documents(&batch20)?;
    let b20 = t.elapsed().as_secs_f64() * 1000.0;

    // Similarity compute cost (pure CPU vector op).
    let qe = backend.embed_query(&queries[0])?;
    let de = backend.embed_document(&docs[0])?;
    let iters = 100_000;
    let t = Instant::now();
    let mut acc = 0.0f32;
    for _ in 0..iters {
        acc += cosine_similarity(&qe, &de);
    }
    let sim_ns = t.elapsed().as_secs_f64() * 1e9 / iters as f64;
    std::hint::black_box(acc);

    println!(
        "\n# LATENCY (n={} queries, {} docs)",
        q_ms.len(),
        d_ms.len()
    );
    println!("QUERY_EMBED_P50_MS = {:.3}", pctl(&q_ms, 50.0));
    println!("QUERY_EMBED_P95_MS = {:.3}", pctl(&q_ms, 95.0));
    println!("DOCUMENT_EMBED_P50_MS = {:.3}", pctl(&d_ms, 50.0));
    println!("DOCUMENT_EMBED_P95_MS = {:.3}", pctl(&d_ms, 95.0));
    println!("BATCH_10_DOCUMENTS_MS = {b10:.3}");
    println!("BATCH_20_DOCUMENTS_MS = {b20:.3}");
    println!("SIMILARITY_COMPUTE_NS_PER_PAIR = {sim_ns:.1}");

    // End-to-end relevance latency: embed query + embed a 10-doc result set +
    // 10 similarities.
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
    println!(
        "END_TO_END_RELEVANCE_P50_MS_10DOCS = {:.3}",
        pctl(&e2e, 50.0)
    );
    println!(
        "END_TO_END_RELEVANCE_P95_MS_10DOCS = {:.3}",
        pctl(&e2e, 95.0)
    );

    Ok(())
}

fn uname() -> String {
    std::fs::read_to_string("/proc/sys/kernel/osrelease")
        .unwrap_or_default()
        .trim()
        .to_string()
}
