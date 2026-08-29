//! STEP 4B §7 — semantic parity check: run the SAME A/B/C/D diagnostic as
//! `diagnostic.rs` but with the Candle backend supplying similarities, on the
//! CONSUMED corpora only. Purpose is parity vs the ONNX prototype, NOT tuning
//! and NOT a generalization claim. Thresholds are reused from the ONNX sweep
//! (passed in) rather than re-swept against Candle.
//!
//! Usage:
//!   cargo run --release --bin candle_diagnostic
//!   cargo run --release --bin candle_diagnostic -- 0.68 0.50   # t_high t_med

use amatl_embedding_relevance_experiment::candle_backend::CandleBackend;
use amatl_embedding_relevance_experiment::corpus::{self, Bucket, Sample};
use amatl_embedding_relevance_experiment::hybrid;
use amatl_embedding_relevance_experiment::{
    cosine_similarity, deterministic, lexical, Confusion, EmbeddingBackend, Label,
};

struct Row {
    sample: Sample,
    label: Label,
    sim: f32,
    doc_empty: bool,
}

fn eval<F: Fn(&Row) -> Label>(rows: &[Row], f: F) -> Confusion {
    let mut c = Confusion::default();
    for r in rows {
        c.record(r.label, f(r));
    }
    c
}

fn bucket_counts<F: Fn(&Row) -> Label>(rows: &[Row], b: Bucket, f: &F) -> (usize, usize) {
    let mut correct = 0;
    let mut total = 0;
    for r in rows {
        if Bucket::classify(&r.sample) == b {
            total += 1;
            if f(r) == r.label {
                correct += 1;
            }
        }
    }
    (correct, total)
}

fn summarize(name: &str, c: &Confusion) {
    let (rp, rr, rf) = c.precision_recall_f1(Label::Relevant);
    println!(
        "{name:<26} acc={:.4} macroF1={:.4} R[P={:.3} R={:.3} F1={:.3}] NR->R={:.3}",
        c.accuracy(),
        c.macro_f1(),
        rp,
        rr,
        rf,
        c.nr_to_r_rate()
    );
}

fn main() -> anyhow::Result<()> {
    let nums: Vec<f32> = std::env::args()
        .skip(1)
        .filter(|a| a != "--")
        .filter_map(|s| s.parse().ok())
        .collect();
    let t_high: f32 = nums.first().copied().unwrap_or(0.72);
    let t_med: f32 = nums.get(1).copied().unwrap_or(0.55);

    let backend = CandleBackend::bge_small_en_v15()?;
    println!("MODEL = {} (dim {})", backend.id(), backend.dim());
    println!("REUSED THRESHOLDS t_high={t_high:.2} t_med={t_med:.2}\n");

    let mut rows: Vec<Row> = Vec::new();
    for path in ["corpus.json", "holdout.json"] {
        let cor = corpus::load(corpus::fixtures_dir().join(path))?;
        let docs: Vec<String> = cor.samples.iter().map(|s| s.document_text()).collect();
        let qs: Vec<String> = cor.samples.iter().map(|s| s.query.clone()).collect();
        let dvs = backend.embed_documents(&docs)?;
        for (i, s) in cor.samples.into_iter().enumerate() {
            let Some(label) = s.label() else { continue };
            let doc_empty = docs[i].trim().is_empty();
            let qv = backend.embed_query(&qs[i])?;
            let sim = cosine_similarity(&qv, &dvs[i]);
            rows.push(Row {
                sample: s,
                label,
                sim,
                doc_empty,
            });
        }
    }
    println!("Total labeled rows: {}\n", rows.len());

    // similarity separation
    let (mut para, mut coll) = (Vec::new(), Vec::new());
    for r in &rows {
        match Bucket::classify(&r.sample) {
            Bucket::ParaphraseRelevant => para.push(r.sim),
            Bucket::LexicalCollisionNegative => coll.push(r.sim),
            _ => {}
        }
    }
    let mean = |v: &[f32]| {
        if v.is_empty() {
            f32::NAN
        } else {
            v.iter().sum::<f32>() / v.len() as f32
        }
    };
    para.sort_by(|a, b| a.partial_cmp(b).unwrap());
    coll.sort_by(|a, b| a.partial_cmp(b).unwrap());
    println!("# SIMILARITY SEPARATION (Candle)");
    println!(
        "PARAPHRASE_RELEVANT   n={:3} mean={:.3} min={:.3}",
        para.len(),
        mean(&para),
        para.first().copied().unwrap_or(f32::NAN)
    );
    println!(
        "LEXICAL_COLLISION_NEG n={:3} mean={:.3} max={:.3}",
        coll.len(),
        mean(&coll),
        coll.last().copied().unwrap_or(f32::NAN)
    );

    let th = t_high;
    let tm = t_med;
    println!("\n# MODE COMPARISON (all labeled consumed rows, Candle sims)");
    let a = eval(&rows, |r| {
        lexical::classify(&r.sample.query, &r.sample.document_text())
    });
    let b = eval(&rows, |r| {
        deterministic::assess(&r.sample.query, &r.sample.document_text()).label
    });
    let c = eval(&rows, |r| {
        hybrid::embedding_only(r.sim, r.doc_empty, th, tm)
    });
    let d = eval(&rows, |r| {
        hybrid::hybrid(&r.sample.query, &r.sample.document_text(), r.sim, th, tm)
            .experimental_final_assessment
    });
    summarize("A_LEXICAL_BASELINE", &a);
    summarize("B_BOUNDED_SEMANTIC", &b);
    summarize("C_EMBEDDING_ONLY", &c);
    summarize("D_HYBRID", &d);

    println!("\n# PER-BUCKET (correct/total) — Candle");
    type ModeFn = Box<dyn Fn(&Row) -> Label>;
    let modes: Vec<(&str, ModeFn)> = vec![
        (
            "A",
            Box::new(|r: &Row| lexical::classify(&r.sample.query, &r.sample.document_text())),
        ),
        (
            "B",
            Box::new(|r: &Row| {
                deterministic::assess(&r.sample.query, &r.sample.document_text()).label
            }),
        ),
        (
            "C",
            Box::new(move |r: &Row| hybrid::embedding_only(r.sim, r.doc_empty, th, tm)),
        ),
        (
            "D",
            Box::new(move |r: &Row| {
                hybrid::hybrid(&r.sample.query, &r.sample.document_text(), r.sim, th, tm)
                    .experimental_final_assessment
            }),
        ),
    ];
    let buckets = [
        Bucket::ParaphraseRelevant,
        Bucket::LexicalCollisionNegative,
        Bucket::EntityMismatch,
        Bucket::Partial,
        Bucket::InsufficientEvidence,
        Bucket::Other,
    ];
    print!("{:<28}", "bucket");
    for (n, _) in &modes {
        print!("{n:>10}");
    }
    println!();
    for bk in buckets {
        print!("{:<28}", bk.name());
        for (_, f) in &modes {
            let (cor, tot) = bucket_counts(&rows, bk, f);
            print!("{:>10}", format!("{cor}/{tot}"));
        }
        println!();
    }

    Ok(())
}
