//! Core-only listing timings on the synthetic folders (no IPC, no UI): time to the first batch
//! and to the whole folder, median of several runs. The criterion benchmarks (#44) replace it.
//!
//! ```text
//! cargo run -p jerry-core --release --example synth
//! cargo run -p jerry-core --release --example listing_bench
//! ```

#![allow(clippy::print_stdout, reason = "a command-line tool")]

use std::sync::atomic::AtomicBool;
use std::time::Instant;

use jerry_core::listing::list;
use jerry_core::synth;

const RUNS: usize = 9;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!(
        "{:>7} | {:>9} {:>9} {:>8} | median of {RUNS}, ms",
        "entries", "1st batch", "all", "batches"
    );
    for count in synth::SIZES {
        let dir = synth::ensure(count)?;
        let mut runs = Vec::new();
        for _ in 0..RUNS {
            let started = Instant::now();
            let mut first = None;
            let mut listed = 0;
            let mut batches = 0;
            list(&dir, &AtomicBool::new(false), |batch| {
                first.get_or_insert_with(|| ms(started));
                listed += batch.len();
                batches += 1;
            })?;
            assert_eq!(listed, count);
            runs.push((first.unwrap_or_default(), ms(started), batches));
        }
        let median = |pick: fn(&(f64, f64, usize)) -> f64| {
            let mut values: Vec<f64> = runs.iter().map(pick).collect();
            values.sort_by(f64::total_cmp);
            values[values.len() / 2]
        };
        println!(
            "{count:>7} | {:>9.2} {:>9.2} {:>8} |",
            median(|run| run.0),
            median(|run| run.1),
            median(|run| run.2 as f64)
        );
    }
    Ok(())
}

fn ms(started: Instant) -> f64 {
    started.elapsed().as_secs_f64() * 1000.0
}
