//! Spike #12: core-only timings for listing and sorting the synthetic folders (no IPC, no UI).
//! The criterion benchmarks (#44) replace this.
//!
//! ```text
//! cargo run -p jerry-core --release --example synth
//! cargo run -p jerry-core --release --example listing_bench
//! ```

#![allow(clippy::print_stdout, reason = "a command-line tool")]

use std::sync::atomic::AtomicBool;
use std::time::Instant;

use jerry_core::listing::list_dir;
use jerry_core::sort::{SortColumn, SortIndex};
use jerry_core::synth;

const RUNS: usize = 7;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!(
        "{:>7} | {:>8} {:>8} | {:>7} {:>9} {:>7} | {:>7} {:>7} {:>7} {:>7} | median of {RUNS} ms",
        "entries", "list", "1st bat", "keys", "name sort", "ranks", "name", "size", "date", "type"
    );
    for count in synth::SIZES {
        let dir = synth::ensure(count)?;
        let mut rows = Vec::new();
        for _ in 0..RUNS {
            let started = Instant::now();
            let mut first_batch = None;
            let entries = list_dir(&dir, &AtomicBool::new(false), |_, _| {
                first_batch.get_or_insert_with(|| ms(started));
            })?;
            let list = ms(started);
            assert_eq!(entries.len(), count);

            let index = SortIndex::build(&entries);
            let mut sorts = [0.0; 4];
            let columns = [
                SortColumn::Name,
                SortColumn::Size,
                SortColumn::Modified,
                SortColumn::Type,
            ];
            for (slot, column) in sorts.iter_mut().zip(columns) {
                let started = Instant::now();
                let order = index.order(&entries, column, true, true);
                *slot = ms(started);
                assert_eq!(order.len(), count);
            }
            rows.push([
                list,
                first_batch.unwrap_or_default(),
                index.timings.keys_ms,
                index.timings.name_sort_ms,
                index.timings.ranks_ms,
                sorts[0],
                sorts[1],
                sorts[2],
                sorts[3],
            ]);
        }
        let median = |column: usize| {
            let mut values: Vec<f64> = rows.iter().map(|row| row[column]).collect();
            values.sort_by(f64::total_cmp);
            values[values.len() / 2]
        };
        println!(
            "{count:>7} | {:>8.2} {:>8.2} | {:>7.2} {:>9.2} {:>7.2} | {:>7.2} {:>7.2} {:>7.2} {:>7.2}",
            median(0),
            median(1),
            median(2),
            median(3),
            median(4),
            median(5),
            median(6),
            median(7),
            median(8)
        );
    }
    Ok(())
}

fn ms(started: Instant) -> f64 {
    started.elapsed().as_secs_f64() * 1000.0
}
