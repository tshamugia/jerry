//! Search benchmarks on the synthetic tree (#65): 50 000 entries in 1 250 nested folders, core
//! only (no IPC, no UI). The PRD's budget is first results in 50 000 files in under a second.
//! The tree is built once under `%TEMP%\jerry-synth\tree-50000` and reused.
//!
//! - `first_batch`: from the call to the first batch with results, for a Georgian word that
//!   about one name in twenty has. The search is cancelled right after it.
//! - `name`: the whole tree, for that word and a wildcard (`ნახაზი *.dwg`).
//! - `filters`: the whole tree for documents of 100 bytes to 1 MB changed since 2020: no name,
//!   so only the filters are asked about every file.
//! - `everything`: the whole tree, every entry a result: the walk and the batches.
//!
//! ```text
//! cargo bench -p jerry-core --bench search
//! ```

#![allow(
    clippy::expect_used,
    reason = "a benchmark: a failed setup aborts the run"
)]

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use criterion::{BenchmarkId, Criterion, SamplingMode, criterion_group, criterion_main};
use jerry_core::search::{DateRange, FileType, SearchQuery, SizeRange, search};
use jerry_core::synth;

/// 2020-01-01T00:00:00Z, in ms.
const YEAR_2020: f64 = 1_577_836_800_000.0;

fn searches(c: &mut Criterion) {
    let count = synth::TREE_SIZE;
    let dir = synth::ensure_tree(count).expect("build the synthetic tree");
    let mut group = c.benchmark_group("search");
    // Tens of ms per run: fewer, equally long samples.
    group.sample_size(20).sampling_mode(SamplingMode::Flat);

    let word = query("ნახაზი");
    group.bench_with_input(BenchmarkId::new("first_batch", count), &dir, |b, dir| {
        b.iter_custom(|runs| (0..runs).map(|_| first_batch(dir, &word)).sum());
    });
    let named = query("ნახაზი *.dwg");
    group.bench_with_input(BenchmarkId::new("name", count), &dir, |b, dir| {
        b.iter(|| whole(dir, &named));
    });
    let filtered = SearchQuery {
        file_type: Some(FileType::Documents),
        size: Some(SizeRange {
            min: Some(100.0),
            max: Some(1e6),
        }),
        modified: Some(DateRange {
            from: Some(YEAR_2020),
            to: None,
        }),
        ..SearchQuery::default()
    };
    group.bench_with_input(BenchmarkId::new("filters", count), &dir, |b, dir| {
        b.iter(|| whole(dir, &filtered));
    });
    let everything = query("");
    group.bench_with_input(BenchmarkId::new("everything", count), &dir, |b, dir| {
        b.iter(|| assert!(whole(dir, &everything) >= count as u64));
    });
    group.finish();
}

fn query(name: &str) -> SearchQuery {
    SearchQuery {
        name: name.to_owned(),
        ..SearchQuery::default()
    }
}

/// Searches all of `dir`; returns how many results there were.
fn whole(dir: &Path, query: &SearchQuery) -> u64 {
    let progress =
        search(dir, query, &[], &AtomicBool::new(false), |_, _| {}).expect("search the tree");
    assert!(progress.found > 0);
    progress.found
}

/// The time from the call to the first batch with results. The search is cancelled right after.
fn first_batch(dir: &Path, query: &SearchQuery) -> Duration {
    let cancel = AtomicBool::new(false);
    let started = Instant::now();
    let mut first = None;
    // Cancelled after the first batch: the error is expected.
    let _ = search(dir, query, &[], &cancel, |batch, _| {
        if !batch.is_empty() {
            first.get_or_insert_with(|| started.elapsed());
            cancel.store(true, Ordering::Relaxed);
        }
    });
    first.expect("the tree gives a batch")
}

criterion_group! {
    name = benches;
    config = Criterion::default()
        .warm_up_time(Duration::from_secs(1))
        .measurement_time(Duration::from_secs(3));
    targets = searches
}
criterion_main!(benches);
