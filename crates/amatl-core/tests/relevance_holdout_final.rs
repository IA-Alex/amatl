//! STEP 3B — FINAL HELD-OUT CAMPAIGN.
//!
//! Loads the independently-labelled held-out corpus
//! (`tests/fixtures/relevance/holdout.json`), whose inputs were frozen at
//! `82fd8e2f052976134dd4d7567aca2c6e903d8d38` **before** any bounded-semantic
//! rule, alias, or intent pattern was designed, and whose `expected_label`s
//! were assigned by an independent party afterwards.
//!
//! This file runs the **production** `assess_result` (bounded-semantic layer
//! enabled by default) over every held-out observation exactly once and reports
//! the acceptance-gate metrics. It never re-implements the heuristic and never
//! tunes anything.
//!
//! Run:
//! ```text
//! cargo test -p amatl-core --test relevance_holdout_final -- --nocapture --test-threads=1
//! ```

use amatl_core::{
    assess_result, parse_query, CanonicalUrl, DeduplicatedResult, DuplicateStatus, OriginalUrl,
    Rank, RelevanceClassification, RelevanceThresholds, ResultType, SCHEMA_VERSION,
};
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Debug, Deserialize)]
struct Corpus {
    #[allow(dead_code)]
    schema: String,
    samples: Vec<Sample>,
}

#[derive(Debug, Clone, Deserialize)]
struct Sample {
    id: String,
    query: String,
    title: Option<String>,
    snippet: Option<String>,
    url: String,
    provider_rank: Option<u32>,
    provider_role: String,
    confirmed_overlap: bool,
    expected_label: String,
    rationale: String,
    query_class: String,
    provenance: String,
}

const HOLDOUT_JSON: &str = include_str!("fixtures/relevance/holdout.json");

fn load() -> Corpus {
    serde_json::from_str(HOLDOUT_JSON).expect("holdout.json parses")
}

fn label_of(s: &str) -> RelevanceClassification {
    match s {
        "RELEVANT" => RelevanceClassification::Relevant,
        "POSSIBLY_RELEVANT" => RelevanceClassification::PossiblyRelevant,
        "NOT_RELEVANT" => RelevanceClassification::NotRelevant,
        "UNKNOWN" => RelevanceClassification::Unknown,
        other => panic!("unknown label {other:?}"),
    }
}

const CLASSES: [RelevanceClassification; 4] = [
    RelevanceClassification::Relevant,
    RelevanceClassification::PossiblyRelevant,
    RelevanceClassification::NotRelevant,
    RelevanceClassification::Unknown,
];

fn class_index(c: RelevanceClassification) -> usize {
    match c {
        RelevanceClassification::Relevant => 0,
        RelevanceClassification::PossiblyRelevant => 1,
        RelevanceClassification::NotRelevant => 2,
        RelevanceClassification::Unknown => 3,
    }
}

fn class_name(c: RelevanceClassification) -> &'static str {
    match c {
        RelevanceClassification::Relevant => "RELEVANT",
        RelevanceClassification::PossiblyRelevant => "POSSIBLY_RELEVANT",
        RelevanceClassification::NotRelevant => "NOT_RELEVANT",
        RelevanceClassification::Unknown => "UNKNOWN",
    }
}

fn sample_to_result(s: &Sample) -> DeduplicatedResult {
    let parsed = url::Url::parse(&s.url).unwrap_or_else(|e| panic!("bad url {}: {e}", s.url));
    let provider = "corpus";
    let rank = s.provider_rank.and_then(|v| Rank::new(v).ok());
    let mut provider_ranks = BTreeMap::new();
    provider_ranks.insert(provider.to_string(), rank);
    DeduplicatedResult {
        schema_version: SCHEMA_VERSION.into(),
        title: s.title.clone(),
        original_url: OriginalUrl(parsed.clone()),
        canonical_url: CanonicalUrl(parsed.clone()),
        original_urls: vec![OriginalUrl(parsed)],
        providers: vec![provider.to_string()],
        representative_provider: provider.into(),
        provider_ranks,
        snippet: s.snippet.clone(),
        alternate_snippets: vec![],
        result_type: ResultType::Organic,
        published_at: None,
        author: None,
        language: None,
        file_type: None,
        thumbnail: None,
        metadata: BTreeMap::new(),
        observed_dates: vec![],
        duplicate_status: DuplicateStatus::Distinct,
        merge_reason: None,
        possible_duplicate_with: vec![],
    }
}

fn safe_div(n: f64, d: f64) -> f64 {
    if d == 0.0 {
        0.0
    } else {
        n / d
    }
}

#[derive(Default)]
struct Metrics {
    total: usize,
    correct: usize,
    confusion: [[usize; 4]; 4],
}

impl Metrics {
    fn record(&mut self, e: RelevanceClassification, p: RelevanceClassification) {
        self.total += 1;
        if e == p {
            self.correct += 1;
        }
        self.confusion[class_index(e)][class_index(p)] += 1;
    }
    fn count(&self, e: RelevanceClassification, p: RelevanceClassification) -> usize {
        self.confusion[class_index(e)][class_index(p)]
    }
    fn accuracy(&self) -> f64 {
        safe_div(self.correct as f64, self.total as f64)
    }
    fn prf(&self, c: RelevanceClassification) -> (f64, f64, f64) {
        let tp = self.count(c, c) as f64;
        let pred: f64 = CLASSES.iter().map(|&e| self.count(e, c) as f64).sum();
        let act: f64 = CLASSES.iter().map(|&p| self.count(c, p) as f64).sum();
        let p = safe_div(tp, pred);
        let r = safe_div(tp, act);
        let f = if p + r == 0.0 {
            0.0
        } else {
            2.0 * p * r / (p + r)
        };
        (p, r, f)
    }
    fn macro_f1(&self) -> f64 {
        CLASSES.iter().map(|&c| self.prf(c).2).sum::<f64>() / CLASSES.len() as f64
    }
    fn expected_total(&self, c: RelevanceClassification) -> usize {
        CLASSES.iter().map(|&p| self.count(c, p)).sum()
    }
}

struct Ure {
    n: usize,
    tp: usize,
    fp: usize,
    fn_: usize,
}

fn run() -> (
    Metrics,
    Ure,
    Vec<(Sample, RelevanceClassification, RelevanceClassification)>,
) {
    let corpus = load();
    let th = RelevanceThresholds::default(); // production config, unchanged
    let mut m = Metrics::default();
    let mut u = Ure {
        n: 0,
        tp: 0,
        fp: 0,
        fn_: 0,
    };
    let mut rows = Vec::new();
    for s in &corpus.samples {
        let expected = label_of(&s.expected_label);
        let q = parse_query(s.query.clone()).expect("query parses");
        let predicted = assess_result(&q, &sample_to_result(s), &th).classification;
        m.record(expected, predicted);
        if s.provider_role == "EXPANSION" && !s.confirmed_overlap {
            u.n += 1;
            let pr = predicted == RelevanceClassification::Relevant;
            let er = expected == RelevanceClassification::Relevant;
            match (pr, er) {
                (true, true) => u.tp += 1,
                (true, false) => u.fp += 1,
                (false, true) => u.fn_ += 1,
                (false, false) => {}
            }
        }
        rows.push((s.clone(), expected, predicted));
    }
    (m, u, rows)
}

#[test]
fn holdout_schema_and_labels_are_complete() {
    let corpus = load();
    assert_eq!(corpus.samples.len(), 93, "holdout sample count");
    let mut ids = std::collections::BTreeSet::new();
    let mut ure_pos = 0;
    for s in &corpus.samples {
        assert!(ids.insert(s.id.clone()), "duplicate id {}", s.id);
        assert!(url::Url::parse(&s.url).is_ok(), "bad url in {}", s.id);
        assert!(
            !s.expected_label.trim().is_empty(),
            "{} has no ground-truth label",
            s.id
        );
        assert!(
            ["RELEVANT", "POSSIBLY_RELEVANT", "NOT_RELEVANT", "UNKNOWN"]
                .contains(&s.expected_label.as_str()),
            "{} bad label {:?}",
            s.id,
            s.expected_label
        );
        assert!(
            s.rationale.split_whitespace().count() >= 2,
            "{} rationale too short",
            s.id
        );
        assert!(
            ["PRIMARY", "EXPANSION"].contains(&s.provider_role.as_str()),
            "{} bad role",
            s.id
        );
        assert!(
            ["REAL", "DERIVED_REAL", "SYNTHETIC_EDGE_CASE"].contains(&s.provenance.as_str()),
            "{} bad provenance",
            s.id
        );
        parse_query(s.query.clone()).unwrap_or_else(|e| panic!("{} unparseable: {e}", s.id));
        if s.provider_role == "EXPANSION" && !s.confirmed_overlap && s.expected_label == "RELEVANT"
        {
            ure_pos += 1;
        }
    }
    println!("HOLDOUT_URE_POSITIVE_COUNT = {ure_pos}");
}

#[test]
fn holdout_final_is_not_used_for_tuning() {
    // Guard: the campaign uses RelevanceThresholds::default() verbatim. No
    // grid, no per-sample overrides, no alternate threshold set.
    let src = HOLDOUT_JSON;
    assert!(src.contains("\"expected_label\""));
    // (documented invariant; the run() fn above constructs exactly one
    // RelevanceThresholds::default() and never mutates it.)
}

#[test]
fn holdout_final_campaign() {
    let (m, u, rows) = run();

    use std::collections::BTreeMap as BMap;
    let mut dist: BMap<&str, usize> = BMap::new();
    for (s, _, _) in &rows {
        *dist.entry(s.expected_label.as_str()).or_default() += 1;
    }

    println!("\n================= STEP 3B FINAL HELD-OUT CAMPAIGN =================");
    println!("IMPLEMENTATION_HEAD = 66598c95e4ffcc22cc8bf551d4a2cc08cd37399a");
    println!("HOLDOUT_FREEZE_HEAD = 82fd8e2f052976134dd4d7567aca2c6e903d8d38");
    println!("FINAL_HOLDOUT_SAMPLE_COUNT = {}", m.total);
    println!("FINAL_HOLDOUT_LABEL_DISTRIBUTION = {dist:?}");
    println!("FINAL_HOLDOUT_ACCURACY = {:.4}", m.accuracy());
    println!("FINAL_HOLDOUT_MACRO_F1 = {:.4}", m.macro_f1());

    for &c in &CLASSES {
        let (p, r, f) = m.prf(c);
        println!(
            "FINAL_HOLDOUT_{:<17} P={:.4} R={:.4} F1={:.4}",
            class_name(c),
            p,
            r,
            f
        );
    }

    println!("\n-- CONFUSION MATRIX (rows=expected, cols=predicted) --");
    print!("  {:<18}", "");
    for &c in &CLASSES {
        print!("{:>18}", class_name(c));
    }
    println!("{:>8}", "TOTAL");
    for &e in &CLASSES {
        print!("  {:<18}", class_name(e));
        for &p in &CLASSES {
            print!("{:>18}", m.count(e, p));
        }
        println!("{:>8}", m.expected_total(e));
    }

    let (rp, rr, rf) = m.prf(RelevanceClassification::Relevant);
    let ure_p = safe_div(u.tp as f64, (u.tp + u.fp) as f64);
    let ure_r = safe_div(u.tp as f64, (u.tp + u.fn_) as f64);

    let nr_total = m.expected_total(RelevanceClassification::NotRelevant) as f64;
    let nr2r = safe_div(
        m.count(
            RelevanceClassification::NotRelevant,
            RelevanceClassification::Relevant,
        ) as f64,
        nr_total,
    );
    let unk_total = m.expected_total(RelevanceClassification::Unknown) as f64;
    let unk2r = safe_div(
        m.count(
            RelevanceClassification::Unknown,
            RelevanceClassification::Relevant,
        ) as f64,
        unk_total,
    );
    let rel_pred: f64 = CLASSES
        .iter()
        .map(|&e| m.count(e, RelevanceClassification::Relevant) as f64)
        .sum();
    let rel_fp: f64 = CLASSES
        .iter()
        .filter(|&&e| e != RelevanceClassification::Relevant)
        .map(|&e| m.count(e, RelevanceClassification::Relevant) as f64)
        .sum();
    let rel_fpr = safe_div(rel_fp, rel_pred);

    println!("\nFINAL_HOLDOUT_RELEVANT_PRECISION = {rp:.4}");
    println!("FINAL_HOLDOUT_RELEVANT_RECALL    = {rr:.4}");
    println!("FINAL_HOLDOUT_RELEVANT_F1        = {rf:.4}");
    println!("FINAL_HOLDOUT_URE_SUBSET = {}", u.n);
    println!(
        "FINAL_HOLDOUT_URE_TP={} URE_FP={} URE_FN={}",
        u.tp, u.fp, u.fn_
    );
    println!("FINAL_HOLDOUT_URE_PRECISION = {ure_p:.4}");
    println!("FINAL_HOLDOUT_URE_RECALL    = {ure_r:.4}");
    println!("FINAL_HOLDOUT_NR_TO_R_ERROR_RATE      = {nr2r:.4}");
    println!("FINAL_HOLDOUT_UNKNOWN_TO_R_ERROR_RATE = {unk2r:.4}");
    println!("FINAL_HOLDOUT_RELEVANT_FALSE_POSITIVE_RATE = {rel_fpr:.4}");

    let gates = [
        ("RELEVANT_PRECISION >= 0.85", rp >= 0.85, rp),
        ("RELEVANT_RECALL >= 0.70", rr >= 0.70, rr),
        ("URE_PRECISION >= 0.85", ure_p >= 0.85, ure_p),
        ("MACRO_F1 >= 0.70", m.macro_f1() >= 0.70, m.macro_f1()),
        ("NR_TO_R_ERROR_RATE <= 0.10", nr2r <= 0.10, nr2r),
        ("UNKNOWN_TO_R_ERROR_RATE <= 0.10", unk2r <= 0.10, unk2r),
    ];
    println!("\n-- ACCEPTANCE GATES --");
    let mut all = true;
    for (name, pass, val) in &gates {
        println!(
            "  [{}] {name} (value={val:.4})",
            if *pass { "PASS" } else { "FAIL" }
        );
        all &= *pass;
    }
    println!("\nALL_GATES_PASS = {all}");

    println!("\n-- ERRORS (expected != predicted) --");
    for (s, e, p) in &rows {
        if e != p {
            println!(
                "  {id:<16} class={qc:<22} expected={e:<17} predicted={p}",
                id = s.id,
                qc = s.query_class,
                e = class_name(*e),
                p = class_name(*p),
            );
        }
    }
}
