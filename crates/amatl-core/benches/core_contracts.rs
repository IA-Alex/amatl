use amatl_core::{
    canonical::canonicalize, classify, parse_query, FieldProvenance, NormalizedResult, OriginalUrl,
    ResultType, SCHEMA_VERSION,
};
use criterion::{criterion_group, criterion_main, Criterion};
use std::collections::BTreeMap;
use std::hint::black_box;

fn benchmark_contracts(criterion: &mut Criterion) {
    criterion.bench_function("query_parse_and_classify", |bencher| {
        bencher.iter(|| {
            let query = parse_query(black_box(
                "rust async runtime site:docs.rs lang:en after:2025-01-01".into(),
            ))
            .unwrap();
            black_box(classify(&query));
        });
    });

    criterion.bench_function("canonicalization", |bencher| {
        let url =
            url::Url::parse("https://EXAMPLE.com:443/path%2fitem?utm_source=bench&id=42#section")
                .unwrap();
        bencher.iter(|| {
            black_box(canonicalize(NormalizedResult {
                schema_version: SCHEMA_VERSION.into(),
                title: None,
                raw_url: url.as_str().into(),
                url: OriginalUrl(url.clone()),
                provider: "benchmark".into(),
                provider_rank: None,
                snippet: None,
                result_type: ResultType::Organic,
                published_at: None,
                author: None,
                language: None,
                file_type: None,
                thumbnail: None,
                metadata: BTreeMap::new(),
                provenance: BTreeMap::from([("url".into(), FieldProvenance::Reported)]),
                degradations: vec![],
            }));
        });
    });
}

criterion_group!(benches, benchmark_contracts);
criterion_main!(benches);
