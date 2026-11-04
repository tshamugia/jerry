//! Listing benchmarks on the synthetic folders (#44), core only (no IPC, no UI): the whole
//! folder, and the time to its first batch, the rows the user sees first. The folders are built
//! once under `%TEMP%\jerry-synth` and reused.
//!
//! ```text
//! cargo bench -p jerry-core --bench listing
//! ```

#![allow(
    clippy::expect_used,
    reason = "a benchmark: a failed setup aborts the run"
)]

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use criterion::{BenchmarkId, Criterion, SamplingMode, criterion_group, criterion_main};
use jerry_core::listing::list;
use jerry_core::synth;

fn listing(c: &mut Criterion) {
    let mut group = c.benchmark_group("listing");
    for count in synth::SIZES {
        let dir = synth::ensure(count).expect("build the synthetic folder");
        if count >= 100_000 {
            // Tens of ms per run: fewer, equally long samples.
            group.sample_size(20).sampling_mode(SamplingMode::Flat);
        }
        group.bench_with_input(BenchmarkId::new("all", count), &dir, |b, dir| {
            b.iter(|| list_all(dir, count));
        });
        group.bench_with_input(BenchmarkId::new("first_batch", count), &dir, |b, dir| {
            b.iter_custom(|runs| (0..runs).map(|_| first_batch(dir)).sum());
        });
    }
    group.finish();
}

fn list_all(dir: &Path, count: usize) {
    let mut listed = 0;
    list(dir, &AtomicBool::new(false), |batch| listed += batch.len()).expect("list the folder");
    assert_eq!(listed, count);
}

/// The time from the call to the first batch. The listing is cancelled right after it.
fn first_batch(dir: &Path) -> Duration {
    let cancel = AtomicBool::new(false);
    let started = Instant::now();
    let mut first = None;
    // Cancelled after the first batch: the error is expected.
    let _ = list(dir, &cancel, |_| {
        first.get_or_insert_with(|| started.elapsed());
        cancel.store(true, Ordering::Relaxed);
    });
    first.expect("the folder gives a batch")
}

criterion_group! {
    name = benches;
    config = Criterion::default()
        .warm_up_time(Duration::from_secs(1))
        .measurement_time(Duration::from_secs(3));
    targets = listing
}
criterion_main!(benches);
