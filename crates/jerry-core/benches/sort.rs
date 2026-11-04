//! Sort benchmarks on the synthetic folders (#44), core only (no IPC, no UI): building the sort
//! index once per listing, then re-sorting by one column of each kind (name rank, `u64` date,
//! type rank), descending with folders first. The folders are built once under
//! `%TEMP%\jerry-synth` and reused.
//!
//! ```text
//! cargo bench -p jerry-core --bench sort
//! ```

#![allow(
    clippy::expect_used,
    reason = "a benchmark: a failed setup aborts the run"
)]

use std::sync::atomic::AtomicBool;
use std::time::Duration;

use criterion::{BenchmarkId, Criterion, SamplingMode, criterion_group, criterion_main};
use jerry_core::listing::Listing;
use jerry_core::sort::{PlainTypeNames, Sort, SortColumn, SortIndex};
use jerry_core::synth;

/// Size and Created sort like Modified (a `u64`), Extension like Type (a rank).
const COLUMNS: [(&str, SortColumn); 3] = [
    ("name", SortColumn::Name),
    ("modified", SortColumn::Modified),
    ("type", SortColumn::Type),
];

fn sort(c: &mut Criterion) {
    let mut group = c.benchmark_group("sort");
    for count in synth::SIZES {
        let dir = synth::ensure(count).expect("build the synthetic folder");
        let listing = Listing::read(&dir, &AtomicBool::new(false)).expect("list the folder");
        let entries = listing.entries();
        if count >= 100_000 {
            // Tens of ms per run: fewer, equally long samples.
            group.sample_size(20).sampling_mode(SamplingMode::Flat);
        }
        group.bench_with_input(BenchmarkId::new("index", count), entries, |b, entries| {
            b.iter(|| SortIndex::build(entries, &PlainTypeNames));
        });
        group.sample_size(100).sampling_mode(SamplingMode::Auto);
        let index = SortIndex::build(entries, &PlainTypeNames);
        for (name, column) in COLUMNS {
            let sort = Sort {
                column,
                descending: true,
                folders_first: true,
            };
            group.bench_with_input(BenchmarkId::new(name, count), &sort, |b, &sort| {
                b.iter(|| index.order(entries, sort));
            });
        }
    }
    group.finish();
}

criterion_group! {
    name = benches;
    config = Criterion::default()
        .warm_up_time(Duration::from_secs(1))
        .measurement_time(Duration::from_secs(3));
    targets = sort
}
criterion_main!(benches);
