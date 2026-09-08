//! A/B/C/D diagnostic on the CONSUMED labeled corpora.
//!
//! This is diagnostic analysis, NOT a generalization claim and NOT a final
//! gate. Thresholds for modes C/D are swept here on the consumed data only.
//!
//! Usage:
//!   cargo run --release --bin diagnostic
//!   cargo run --release --bin diagnostic -- minilm

use amatl_embedding_relevance_experiment::corpus::{self, Bucket, Sample};
use amatl_embedding_relevance_experiment::fastembed_backend::FastembedBackend;
use amatl_embedding_relevance_experiment::hybrid::{self};
use amatl_embedding_relevance_experiment::{
    cosine_similarity, deterministic, lexical, Confusion, EmbeddingBackend, Label,
};
use std::collections::BTreeMap;

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

fn bucket_accuracy<F: Fn(&Row) -> Label>(rows: &[Row], b: Bucket, f: &F) -> (usize, usize) {
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
    let which = std::env::args().nth(1).unwrap_or_default();
    let backend: FastembedBackend = if which == "minilm" {
        FastembedBackend::all_minilm_l6_v2()?
    } else {
        FastembedBackend::bge_small_en_v15()?
    };
    println!("MODEL = {} (dim {})\n", backend.id(), backend.dim());

    let mut rows: Vec<Row> = Vec::new();
    for path in ["corpus.json", "holdout.json"] {
        let cpath = corpus::fixtures_dir().join(path);
        let cor = corpus::load(&cpath)?;
        println!(
            "# {path}: {} samples (schema {})",
            cor.samples.len(),
            cor.schema
        );
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
    println!("\nTotal labeled rows: {}\n", rows.len());

    // ---- similarity separation: paraphrase-relevant vs lexical-collision-neg
    let mut para: Vec<f32> = Vec::new();
    let mut coll: Vec<f32> = Vec::new();
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
    println!("# SIMILARITY SEPARATION");
    println!(
        "PARAPHRASE_RELEVANT      n={:3} mean={:.3} min={:.3} median={:.3}",
        para.len(),
        mean(&para),
        para.first().copied().unwrap_or(f32::NAN),
        para.get(para.len() / 2).copied().unwrap_or(f32::NAN),
    );
    println!(
        "LEXICAL_COLLISION_NEG    n={:3} mean={:.3} max={:.3} median={:.3}",
        coll.len(),
        mean(&coll),
        coll.last().copied().unwrap_or(f32::NAN),
        coll.get(coll.len() / 2).copied().unwrap_or(f32::NAN),
    );

    // ---- threshold sweep for modes C/D on consumed data
    println!("\n# THRESHOLD SWEEP (consumed data, diagnostic only)");
    let mut best = (0.0f32, 0.0f32, -1.0f64);
    for h in [0.55, 0.60, 0.62, 0.65, 0.68, 0.70, 0.72, 0.75] {
        for m in [0.35, 0.40, 0.45, 0.48, 0.50, 0.52, 0.55] {
            if m >= h {
                continue;
            }
            let c = eval(&rows, |r| {
                hybrid::hybrid(&r.sample.query, &r.sample.document_text(), r.sim, h, m)
                    .experimental_final_assessment
            });
            let score = c.macro_f1();
            if score > best.2 {
                best = (h, m, score);
            }
        }
    }
    println!(
        "BEST_HYBRID_THRESHOLDS t_high={:.2} t_med={:.2} macroF1={:.4}",
        best.0, best.1, best.2
    );
    let (th, tm) = (best.0, best.1);

    // ---- A/B/C/D
    println!("\n# MODE COMPARISON (all labeled consumed rows)");
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

    // ---- per-bucket accuracy for each mode
    println!("\n# PER-BUCKET ACCURACY (correct/total)");
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
        print!("{:>12}", n);
    }
    println!();
    let mut deltas: BTreeMap<&str, (f64, f64)> = BTreeMap::new();
    for bk in buckets {
        print!("{:<28}", bk.name());
        let mut vals = Vec::new();
        for (_, f) in &modes {
            let (cor, tot) = bucket_accuracy(&rows, bk, f);
            let acc = if tot > 0 {
                cor as f64 / tot as f64
            } else {
                f64::NAN
            };
            vals.push(acc);
            print!("{:>10} {:>1}", format!("{cor}/{tot}"), "");
        }
        println!();
        // delta D vs B for this bucket
        deltas.insert(bk.name(), (vals[1], vals[3]));
    }

    println!("\n# KEY DELTAS (D hybrid vs B bounded-semantic), consumed diagnostic");
    for (name, (bv, dv)) in &deltas {
        println!("{name:<28} B={:.3}  D={:.3}  delta={:+.3}", bv, dv, dv - bv);
    }

    Ok(())
}
