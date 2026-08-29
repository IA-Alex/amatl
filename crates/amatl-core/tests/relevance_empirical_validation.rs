//! STEP 3 — RELEVANCE EMPIRICAL VALIDATION.
//!
//! Loads the versioned corpus at `tests/fixtures/relevance/corpus.json`, runs
//! the **production** deterministic heuristic ([`amatl_core::assess_result`])
//! over every observation, and compares the predicted classification against the
//! human ground-truth label. The heuristic logic is never re-implemented here;
//! this file only loads data, invokes `assess_result`, and computes metrics.
//!
//! Ground truth was assigned by human semantic-utility judgement, deliberately
//! independent of the heuristic's lexical mechanism (see the corpus header).
//!
//! Acceptance gates are checked and *reported*, but by default a failing gate
//! does not fail the test run — the baseline is meant to be measured, not
//! defended. Set `AMATL_RELEVANCE_ENFORCE_GATES=1` to make gate failure a hard
//! error (used by a dedicated empirical-validation CI job, not the functional
//! suite).
//!
//! Run just this campaign:
//! ```text
//! cargo test -p amatl-core --test relevance_empirical_validation -- --nocapture
//! ```

use amatl_core::{
    assess_result, parse_query, CanonicalUrl, DeduplicatedResult, DuplicateStatus, OriginalUrl,
    Rank, RelevanceClassification, RelevanceThresholds, ResultType, SCHEMA_VERSION,
};
use serde::Deserialize;
use std::collections::BTreeMap;

// --------------------------------------------------------------------------
// Corpus schema
// --------------------------------------------------------------------------

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

const CORPUS_JSON: &str = include_str!("fixtures/relevance/corpus.json");

fn load_corpus() -> Corpus {
    serde_json::from_str(CORPUS_JSON).expect("corpus.json parses")
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

// --------------------------------------------------------------------------
// Build a DeduplicatedResult from a corpus sample (mirrors the phase-2e helper)
// --------------------------------------------------------------------------

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

fn predict(s: &Sample, thresholds: &RelevanceThresholds) -> RelevanceClassification {
    let query = parse_query(s.query.clone()).expect("query parses");
    let result = sample_to_result(s);
    assess_result(&query, &result, thresholds).classification
}

// --------------------------------------------------------------------------
// Metrics
// --------------------------------------------------------------------------

#[derive(Clone, Default)]
struct Metrics {
    total: usize,
    correct: usize,
    /// confusion[expected][predicted] -> count
    confusion: [[usize; 4]; 4],
}

impl Metrics {
    fn record(&mut self, expected: RelevanceClassification, predicted: RelevanceClassification) {
        self.total += 1;
        if expected == predicted {
            self.correct += 1;
        }
        self.confusion[class_index(expected)][class_index(predicted)] += 1;
    }

    fn count(&self, e: RelevanceClassification, p: RelevanceClassification) -> usize {
        self.confusion[class_index(e)][class_index(p)]
    }

    fn accuracy(&self) -> f64 {
        safe_div(self.correct as f64, self.total as f64)
    }

    /// (precision, recall, f1) for one class.
    fn prf(&self, class: RelevanceClassification) -> (f64, f64, f64) {
        let tp = self.count(class, class) as f64;
        let predicted_pos: f64 = CLASSES.iter().map(|&e| self.count(e, class) as f64).sum();
        let actual_pos: f64 = CLASSES.iter().map(|&p| self.count(class, p) as f64).sum();
        let precision = safe_div(tp, predicted_pos);
        let recall = safe_div(tp, actual_pos);
        let f1 = if precision + recall == 0.0 {
            0.0
        } else {
            2.0 * precision * recall / (precision + recall)
        };
        (precision, recall, f1)
    }

    fn macro_f1(&self) -> f64 {
        let sum: f64 = CLASSES.iter().map(|&c| self.prf(c).2).sum();
        sum / CLASSES.len() as f64
    }

    fn expected_total(&self, class: RelevanceClassification) -> usize {
        CLASSES.iter().map(|&p| self.count(class, p)).sum()
    }

    fn confusion_total(&self) -> usize {
        self.confusion.iter().flatten().sum()
    }
}

fn safe_div(num: f64, den: f64) -> f64 {
    if den == 0.0 {
        0.0
    } else {
        num / den
    }
}

// --------------------------------------------------------------------------
// Severity model (diagnostic only — never feeds the algorithm)
// --------------------------------------------------------------------------

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum Severity {
    Critical,
    High,
    Medium,
    Low,
    None,
}

fn severity(expected: RelevanceClassification, predicted: RelevanceClassification) -> Severity {
    use RelevanceClassification::*;
    if expected == predicted {
        return Severity::None;
    }
    match (expected, predicted) {
        (NotRelevant, Relevant) => Severity::Critical,
        (Unknown, Relevant) => Severity::High,
        (PossiblyRelevant, Relevant) => Severity::Medium,
        (Relevant, NotRelevant) => Severity::High,
        (Relevant, PossiblyRelevant) => Severity::Low,
        (Unknown, NotRelevant) => Severity::Medium,
        (NotRelevant, Unknown) => Severity::Low,
        (PossiblyRelevant, NotRelevant) => Severity::Medium,
        (NotRelevant, PossiblyRelevant) => Severity::Medium,
        (PossiblyRelevant, Unknown) => Severity::Low,
        (Unknown, PossiblyRelevant) => Severity::Low,
        (Relevant, Unknown) => Severity::Medium,
        _ => Severity::Low,
    }
}

// --------------------------------------------------------------------------
// Deterministic stratified split
// --------------------------------------------------------------------------

/// Tiny deterministic hash (FNV-1a) so the 70/30 split is reproducible without
/// pulling in `rand`. Seed is fixed.
fn fnv1a(seed: u64, s: &str) -> u64 {
    let mut h = seed ^ 0xcbf29ce484222325;
    for b in s.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

const SPLIT_SEED: u64 = 0x5445_3350_5f33; // "TE3P_3"

/// Returns true if the sample is in the TUNING split (~70%), false for VALIDATION.
fn in_tuning_split(sample_id: &str) -> bool {
    fnv1a(SPLIT_SEED, sample_id) % 100 < 70
}

// --------------------------------------------------------------------------
// The campaign
// --------------------------------------------------------------------------

struct Outcome {
    sample: Sample,
    predicted: RelevanceClassification,
    expected: RelevanceClassification,
}

fn run(thresholds: &RelevanceThresholds, samples: &[Sample]) -> (Metrics, Vec<Outcome>) {
    let mut m = Metrics::default();
    let mut outcomes = Vec::new();
    for s in samples {
        let expected = label_of(&s.expected_label);
        let predicted = predict(s, thresholds);
        m.record(expected, predicted);
        outcomes.push(Outcome {
            sample: s.clone(),
            predicted,
            expected,
        });
    }
    (m, outcomes)
}

/// Unique-Relevant-Expansion subset metrics: samples with
/// provider_role == EXPANSION and confirmed_overlap == false.
struct UreMetrics {
    n: usize,
    /// predicted RELEVANT & expected RELEVANT
    tp: usize,
    /// predicted RELEVANT & expected != RELEVANT
    fp: usize,
    /// predicted != RELEVANT & expected RELEVANT
    fn_: usize,
    /// expected != RELEVANT
    negatives: usize,
}

fn ure_metrics(outcomes: &[Outcome]) -> UreMetrics {
    let mut u = UreMetrics {
        n: 0,
        tp: 0,
        fp: 0,
        fn_: 0,
        negatives: 0,
    };
    for o in outcomes {
        if o.sample.provider_role != "EXPANSION" || o.sample.confirmed_overlap {
            continue;
        }
        u.n += 1;
        let pred_rel = o.predicted == RelevanceClassification::Relevant;
        let exp_rel = o.expected == RelevanceClassification::Relevant;
        if !exp_rel {
            u.negatives += 1;
        }
        match (pred_rel, exp_rel) {
            (true, true) => u.tp += 1,
            (true, false) => u.fp += 1,
            (false, true) => u.fn_ += 1,
            (false, false) => {}
        }
    }
    u
}

fn print_report(title: &str, m: &Metrics, outcomes: &[Outcome]) {
    println!("\n================= {title} =================");
    println!("TOTAL_SAMPLES = {}", m.total);
    println!("ACCURACY = {:.4}", m.accuracy());
    println!("MACRO_F1 = {:.4}", m.macro_f1());

    println!("\n-- CLASS_DISTRIBUTION_EXPECTED / PREDICTED --");
    for &c in &CLASSES {
        let exp = m.expected_total(c);
        let pred: usize = CLASSES.iter().map(|&e| m.count(e, c)).sum();
        println!(
            "  {:<18} expected={:>3}  predicted={:>3}",
            class_name(c),
            exp,
            pred
        );
    }

    println!("\n-- PER-CLASS PRECISION / RECALL / F1 --");
    for &c in &CLASSES {
        let (p, r, f) = m.prf(c);
        println!(
            "  {:<18} P={:.4}  R={:.4}  F1={:.4}",
            class_name(c),
            p,
            r,
            f
        );
    }

    println!("\n-- CONFUSION MATRIX (rows = expected, cols = predicted) --");
    print!("  {:<18}", "");
    for &c in &CLASSES {
        print!("{:>12}", class_name(c));
    }
    println!("{:>8}", "TOTAL");
    for &e in &CLASSES {
        print!("  {:<18}", class_name(e));
        for &p in &CLASSES {
            print!("{:>12}", m.count(e, p));
        }
        println!("{:>8}", m.expected_total(e));
    }

    // AMATL-critical rates
    let rel_predicted: f64 = CLASSES
        .iter()
        .map(|&e| m.count(e, RelevanceClassification::Relevant) as f64)
        .sum();
    let rel_fp: f64 = CLASSES
        .iter()
        .filter(|&&e| e != RelevanceClassification::Relevant)
        .map(|&e| m.count(e, RelevanceClassification::Relevant) as f64)
        .sum();
    let relevant_false_positive_rate = safe_div(rel_fp, rel_predicted);

    let nr_total = m.expected_total(RelevanceClassification::NotRelevant) as f64;
    let nr_to_rel = m.count(
        RelevanceClassification::NotRelevant,
        RelevanceClassification::Relevant,
    ) as f64;
    let not_relevant_to_relevant_error_rate = safe_div(nr_to_rel, nr_total);

    let unk_total = m.expected_total(RelevanceClassification::Unknown) as f64;
    let unk_to_rel = m.count(
        RelevanceClassification::Unknown,
        RelevanceClassification::Relevant,
    ) as f64;
    let unknown_to_relevant_error_rate = safe_div(unk_to_rel, unk_total);

    let pos_total = m.expected_total(RelevanceClassification::PossiblyRelevant) as f64;
    let pos_to_rel = m.count(
        RelevanceClassification::PossiblyRelevant,
        RelevanceClassification::Relevant,
    ) as f64;
    let possible_to_relevant_rate = safe_div(pos_to_rel, pos_total);

    let nr_fn: f64 = CLASSES
        .iter()
        .filter(|&&p| p != RelevanceClassification::NotRelevant)
        .map(|&p| m.count(RelevanceClassification::NotRelevant, p) as f64)
        .sum();
    let not_relevant_false_negative_rate = safe_div(nr_fn, nr_total);

    println!("\n-- AMATL-CRITICAL RATES --");
    println!("RELEVANT_FALSE_POSITIVE_RATE       = {relevant_false_positive_rate:.4}");
    println!("NOT_RELEVANT_TO_RELEVANT_ERROR_RATE= {not_relevant_to_relevant_error_rate:.4}");
    println!("UNKNOWN_TO_RELEVANT_ERROR_RATE     = {unknown_to_relevant_error_rate:.4}");
    println!("POSSIBLE_TO_RELEVANT_RATE          = {possible_to_relevant_rate:.4}");
    println!("NOT_RELEVANT_FALSE_NEGATIVE_RATE   = {not_relevant_false_negative_rate:.4}");

    // URE subset
    let u = ure_metrics(outcomes);
    let ure_precision = safe_div(u.tp as f64, (u.tp + u.fp) as f64);
    let ure_recall = safe_div(u.tp as f64, (u.tp + u.fn_) as f64);
    let ure_fpr = safe_div(u.fp as f64, u.negatives as f64);
    println!("\n-- UNIQUE RELEVANT EXPANSION (role=EXPANSION, overlap=false) --");
    println!("URE_SUBSET_SIZE          = {}", u.n);
    println!("URE_TP={} URE_FP={} URE_FN={}", u.tp, u.fp, u.fn_);
    println!("URE_PRECISION            = {ure_precision:.4}");
    println!("URE_RECALL               = {ure_recall:.4}");
    println!("URE_FALSE_POSITIVE_RATE  = {ure_fpr:.4}");

    // Severity tally
    let mut crit = Vec::new();
    let mut high = Vec::new();
    let mut med = Vec::new();
    let mut low = Vec::new();
    for o in outcomes {
        match severity(o.expected, o.predicted) {
            Severity::Critical => crit.push(o.sample.id.as_str()),
            Severity::High => high.push(o.sample.id.as_str()),
            Severity::Medium => med.push(o.sample.id.as_str()),
            Severity::Low => low.push(o.sample.id.as_str()),
            Severity::None => {}
        }
    }
    println!("\n-- CONFUSION COST (diagnostic only) --");
    println!("CRITICAL_ERRORS ({}) = {:?}", crit.len(), crit);
    println!("HIGH_ERRORS     ({}) = {:?}", high.len(), high);
    println!("MEDIUM_ERRORS   ({}) = {:?}", med.len(), med);
    println!("LOW_ERRORS      ({}) = {:?}", low.len(), low);

    // Error detail for critical + high
    println!("\n-- ERROR ANALYSIS (CRITICAL + HIGH) --");
    for o in outcomes {
        let sev = severity(o.expected, o.predicted);
        if sev != Severity::Critical && sev != Severity::High {
            continue;
        }
        let query = parse_query(o.sample.query.clone()).unwrap();
        let result = sample_to_result(&o.sample);
        let a = assess_result(&query, &result, &RelevanceThresholds::default());
        let snippet_summary = o
            .sample
            .snippet
            .as_deref()
            .map(|s| s.chars().take(70).collect::<String>())
            .unwrap_or_else(|| "<none>".into());
        println!(
            "  [{sev:?}] {id} q={q:?} class={qc}\n     expected={exp} predicted={pred}\n     title={title:?}\n     snippet~={snip:?}\n     title_cov={tc:.2} snippet_cov={sc:.2} url_cov={uc:.2} exact_phrase={ep} rank_top={rt:?} sufficient_text={st}\n     rationale: {rat}",
            id = o.sample.id,
            q = o.sample.query,
            qc = o.sample.query_class,
            exp = class_name(o.expected),
            pred = class_name(o.predicted),
            title = o.sample.title,
            snip = snippet_summary,
            tc = a.title_term_coverage,
            sc = a.snippet_term_coverage,
            uc = a.url_term_coverage,
            ep = a.exact_phrase_match,
            rt = a.provider_rank_is_top,
            st = a.had_sufficient_text,
            rat = o.sample.rationale,
        );
    }
}

#[test]
fn empirical_baseline_full_corpus() {
    let corpus = load_corpus();
    let (m, outcomes) = run(&RelevanceThresholds::default(), &corpus.samples);

    // Provenance tally
    let mut prov: BTreeMap<&str, usize> = BTreeMap::new();
    let mut qclass: BTreeMap<&str, usize> = BTreeMap::new();
    for s in &corpus.samples {
        *prov.entry(s.provenance.as_str()).or_default() += 1;
        *qclass.entry(s.query_class.as_str()).or_default() += 1;
    }
    println!("PROVENANCE = {prov:?}");
    println!("QUERY_CLASS_DISTRIBUTION = {qclass:?}");

    print_report("BASELINE — FULL CORPUS — default thresholds", &m, &outcomes);

    // Acceptance gates (reported; enforced only under env flag).
    let (rp, rr, _rf) = m.prf(RelevanceClassification::Relevant);
    let u = ure_metrics(&outcomes);
    let ure_precision = safe_div(u.tp as f64, (u.tp + u.fp) as f64);
    let nr_total = m.expected_total(RelevanceClassification::NotRelevant) as f64;
    let nr_to_rel = m.count(
        RelevanceClassification::NotRelevant,
        RelevanceClassification::Relevant,
    ) as f64;
    let nr2r = safe_div(nr_to_rel, nr_total);
    let unk_total = m.expected_total(RelevanceClassification::Unknown) as f64;
    let unk_to_rel = m.count(
        RelevanceClassification::Unknown,
        RelevanceClassification::Relevant,
    ) as f64;
    let unk2r = safe_div(unk_to_rel, unk_total);
    let macro_f1 = m.macro_f1();

    let gates = [
        ("RELEVANT_PRECISION >= 0.85", rp >= 0.85, rp),
        (
            "URE_PRECISION >= 0.85",
            ure_precision >= 0.85,
            ure_precision,
        ),
        (
            "NOT_RELEVANT_TO_RELEVANT_ERROR_RATE <= 0.10",
            nr2r <= 0.10,
            nr2r,
        ),
        (
            "UNKNOWN_TO_RELEVANT_ERROR_RATE <= 0.10",
            unk2r <= 0.10,
            unk2r,
        ),
        ("MACRO_F1 >= 0.70", macro_f1 >= 0.70, macro_f1),
    ];
    println!("\n-- INITIAL_ACCEPTANCE_GATES --");
    let mut all_pass = true;
    for (name, pass, val) in &gates {
        println!(
            "  [{}] {name}  (value={val:.4})",
            if *pass { "PASS" } else { "FAIL" }
        );
        all_pass &= *pass;
    }
    println!("\nRELEVANCE_RECALL (informational) = {rr:.4}");
    println!("CURRENT_THRESHOLDS_PASS_GATES = {all_pass}");

    if std::env::var("AMATL_RELEVANCE_ENFORCE_GATES").as_deref() == Ok("1") {
        assert!(all_pass, "acceptance gates failed (enforced)");
    }
}

#[test]
fn threshold_sensitivity_grid_on_tuning_split() {
    let corpus = load_corpus();
    let tuning: Vec<Sample> = corpus
        .samples
        .iter()
        .filter(|s| in_tuning_split(&s.id))
        .cloned()
        .collect();
    let validation: Vec<Sample> = corpus
        .samples
        .iter()
        .filter(|s| !in_tuning_split(&s.id))
        .cloned()
        .collect();
    println!(
        "SPLIT (seed={SPLIT_SEED:#x}): tuning={} validation={}",
        tuning.len(),
        validation.len()
    );

    let title_grid = [0.60, 0.70, 0.75, 0.80, 0.90];
    let possibly_grid = [0.30, 0.40, 0.50];
    let ceiling_grid = [0.15, 0.25, 0.35];
    let combined_grid = [0.90, 1.0, 1.10];

    let base = RelevanceThresholds::default();
    let mut best: Option<(f64, RelevanceThresholds, String)> = None;

    println!("\n-- THRESHOLD SENSITIVITY (tuning split; selection metric = 0.5*RELEVANT_P + 0.3*URE_P + 0.2*MACRO_F1, gated on nr2r<=0.10) --");
    for &t in &title_grid {
        for &pg in &possibly_grid {
            for &cg in &ceiling_grid {
                for &comb in &combined_grid {
                    let th = RelevanceThresholds {
                        relevant_title_coverage: t,
                        possibly_relevant_coverage: pg,
                        not_relevant_ceiling: cg,
                        relevant_combined_coverage: comb,
                        ..base
                    };
                    let (m, outc) = run(&th, &tuning);
                    let (rp, _rr, _rf) = m.prf(RelevanceClassification::Relevant);
                    let u = ure_metrics(&outc);
                    let ure_p = safe_div(u.tp as f64, (u.tp + u.fp) as f64);
                    let macro_f1 = m.macro_f1();
                    let nr_total = m.expected_total(RelevanceClassification::NotRelevant) as f64;
                    let nr2r = safe_div(
                        m.count(
                            RelevanceClassification::NotRelevant,
                            RelevanceClassification::Relevant,
                        ) as f64,
                        nr_total,
                    );
                    let score = 0.5 * rp + 0.3 * ure_p + 0.2 * macro_f1;
                    let tag = format!(
                        "title={t:.2} possibly={pg:.2} ceiling={cg:.2} combined={comb:.2} -> RELEVANT_P={rp:.3} URE_P={ure_p:.3} MACRO_F1={macro_f1:.3} nr2r={nr2r:.3} score={score:.4}"
                    );
                    if nr2r <= 0.10 {
                        match &best {
                            Some((s, _, _)) if *s >= score => {}
                            _ => best = Some((score, th, tag.clone())),
                        }
                    }
                }
            }
        }
    }

    // Baseline on both splits for reference.
    let (mb_t, ob_t) = run(&base, &tuning);
    let (mb_v, ob_v) = run(&base, &validation);
    let ure_t = ure_metrics(&ob_t);
    let ure_v = ure_metrics(&ob_v);
    println!(
        "\nBASELINE tuning:     RELEVANT_P={:.3} URE_P={:.3} MACRO_F1={:.3}",
        mb_t.prf(RelevanceClassification::Relevant).0,
        safe_div(ure_t.tp as f64, (ure_t.tp + ure_t.fp) as f64),
        mb_t.macro_f1()
    );
    println!(
        "BASELINE validation: RELEVANT_P={:.3} URE_P={:.3} MACRO_F1={:.3}",
        mb_v.prf(RelevanceClassification::Relevant).0,
        safe_div(ure_v.tp as f64, (ure_v.tp + ure_v.fp) as f64),
        mb_v.macro_f1()
    );

    match best {
        Some((score, th, tag)) => {
            println!("\nBEST ON TUNING (score={score:.4}): {tag}");
            // Honest check: evaluate that candidate on the untouched validation split.
            let (mv, ov) = run(&th, &validation);
            let uv = ure_metrics(&ov);
            println!(
                "SAME CANDIDATE ON VALIDATION: RELEVANT_P={:.3} URE_P={:.3} MACRO_F1={:.3} nr2r={:.3}",
                mv.prf(RelevanceClassification::Relevant).0,
                safe_div(uv.tp as f64, (uv.tp + uv.fp) as f64),
                mv.macro_f1(),
                safe_div(
                    mv.count(
                        RelevanceClassification::NotRelevant,
                        RelevanceClassification::Relevant
                    ) as f64,
                    mv.expected_total(RelevanceClassification::NotRelevant) as f64
                )
            );
            println!(
                "RECOMMENDED_THRESHOLDS(candidate) = title={:.2} possibly={:.2} ceiling={:.2} combined={:.2}",
                th.relevant_title_coverage,
                th.possibly_relevant_coverage,
                th.not_relevant_ceiling,
                th.relevant_combined_coverage
            );
        }
        None => println!("\nNo grid point satisfied the nr2r<=0.10 gate on the tuning split."),
    }
}

// --------------------------------------------------------------------------
// Corpus / harness sanity tests
// --------------------------------------------------------------------------

#[test]
fn corpus_schema_is_valid() {
    let corpus = load_corpus();
    assert!(
        corpus.samples.len() >= 80,
        "corpus too small: {}",
        corpus.samples.len()
    );
    let mut ids = std::collections::BTreeSet::new();
    for s in &corpus.samples {
        assert!(ids.insert(s.id.clone()), "duplicate id {}", s.id);
        assert!(url::Url::parse(&s.url).is_ok(), "bad url in {}", s.id);
        assert!(
            ["RELEVANT", "POSSIBLY_RELEVANT", "NOT_RELEVANT", "UNKNOWN"]
                .contains(&s.expected_label.as_str()),
            "bad label in {}",
            s.id
        );
        assert!(
            ["PRIMARY", "EXPANSION"].contains(&s.provider_role.as_str()),
            "bad role in {}",
            s.id
        );
        assert!(
            ["REAL", "DERIVED_REAL", "SYNTHETIC_EDGE_CASE"].contains(&s.provenance.as_str()),
            "bad provenance in {}",
            s.id
        );
        // parseable as a query
        parse_query(s.query.clone()).unwrap_or_else(|e| panic!("query {} unparseable: {e}", s.id));
    }
}

#[test]
fn all_samples_have_ground_truth() {
    for s in &load_corpus().samples {
        assert!(
            !s.expected_label.trim().is_empty(),
            "{} missing label",
            s.id
        );
    }
}

#[test]
fn all_samples_have_rationale() {
    for s in &load_corpus().samples {
        assert!(
            s.rationale.split_whitespace().count() >= 2,
            "{} rationale too short: {:?}",
            s.id,
            s.rationale
        );
        // Ground truth must reflect semantic utility, not lexical overlap.
        let lower = s.rationale.to_lowercase();
        assert!(
            ![
                "matches terms",
                "matches the terms",
                "shares terms",
                "same words"
            ]
            .iter()
            .any(|bad| lower.trim() == *bad),
            "{} rationale is lexical-only: {:?}",
            s.id,
            s.rationale
        );
    }
}

#[test]
fn empirical_baseline_is_reproducible() {
    let corpus = load_corpus();
    let (m1, _) = run(&RelevanceThresholds::default(), &corpus.samples);
    let (m2, _) = run(&RelevanceThresholds::default(), &corpus.samples);
    assert_eq!(m1.total, m2.total);
    assert_eq!(m1.correct, m2.correct);
    assert_eq!(m1.confusion, m2.confusion);
}

#[test]
fn confusion_matrix_totals_match_sample_count() {
    let corpus = load_corpus();
    let (m, _) = run(&RelevanceThresholds::default(), &corpus.samples);
    assert_eq!(m.confusion_total(), corpus.samples.len());
    assert_eq!(m.total, corpus.samples.len());
    let per_class: usize = CLASSES.iter().map(|&c| m.expected_total(c)).sum();
    assert_eq!(per_class, corpus.samples.len());
}

#[test]
fn metrics_have_safe_denominators() {
    // An empty run must not panic or produce NaN.
    let (m, outcomes) = run(&RelevanceThresholds::default(), &[]);
    assert!(m.accuracy().is_finite());
    assert!(m.macro_f1().is_finite());
    for &c in &CLASSES {
        let (p, r, f) = m.prf(c);
        assert!(p.is_finite() && r.is_finite() && f.is_finite());
    }
    let u = ure_metrics(&outcomes);
    assert_eq!(u.n, 0);
}

#[test]
fn unique_relevant_expansion_subset_is_measured() {
    let corpus = load_corpus();
    let (_, outcomes) = run(&RelevanceThresholds::default(), &corpus.samples);
    let u = ure_metrics(&outcomes);
    assert!(u.n >= 10, "URE subset too small to measure: {}", u.n);
}

#[test]
fn fixed_seed_split_is_reproducible() {
    let corpus = load_corpus();
    let a: Vec<bool> = corpus
        .samples
        .iter()
        .map(|s| in_tuning_split(&s.id))
        .collect();
    let b: Vec<bool> = corpus
        .samples
        .iter()
        .map(|s| in_tuning_split(&s.id))
        .collect();
    assert_eq!(a, b);
    let tuning = a.iter().filter(|x| **x).count();
    // Roughly 70/30, allow slack for a small corpus.
    let frac = tuning as f64 / corpus.samples.len() as f64;
    assert!(
        (0.55..0.85).contains(&frac),
        "split fraction off: {frac:.2}"
    );
}
