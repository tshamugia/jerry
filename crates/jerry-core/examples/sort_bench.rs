//! Core-only sort timings on the synthetic folders (no IPC, no UI): building the sort index
//! once, then re-sorting by each column, median of several runs. The criterion benchmarks (#44)
//! replace it.
//!
//! ```text
//! cargo run -p jerry-core --release --example synth
//! cargo run -p jerry-core --release --example sort_bench
//! ```

#![allow(clippy::print_stdout, reason = "a command-line tool")]

use std::sync::atomic::AtomicBool;
use std::time::Instant;

use jerry_core::listing::Listing;
use jerry_core::sort::{PlainTypeNames, Sort, SortColumn, SortIndex};
use jerry_core::synth;

const RUNS: usize = 9;

const COLUMNS: [SortColumn; 6] = [
    SortColumn::Name,
    SortColumn::Modified,
    SortColumn::Type,
    SortColumn::Size,
    SortColumn::Created,
    SortColumn::Extension,
];

fn main() -> Result<(), Box<dyn std::error::Error>> {
    print!("{:>7} | {:>7}", "entries", "index");
    for column in COLUMNS {
        print!(" {:>9}", format!("{column:?}"));
    }
    println!(" | median of {RUNS}, ms; columns: re-sort, descending");
    for count in synth::SIZES {
        let listing = Listing::read(&synth::ensure(count)?, &AtomicBool::new(false))?;
        let entries = listing.entries();
        let index_ms = median((0..RUNS).map(|_| {
            let started = Instant::now();
            std::hint::black_box(SortIndex::build(entries, &PlainTypeNames));
            ms(started)
        }));
        let index = SortIndex::build(entries, &PlainTypeNames);
        print!("{count:>7} | {index_ms:>7.2}");
        for column in COLUMNS {
            let sort = Sort {
                column,
                descending: true,
                folders_first: true,
            };
            let order_ms = median((0..RUNS).map(|_| {
                let started = Instant::now();
                std::hint::black_box(index.order(entries, sort));
                ms(started)
            }));
            print!(" {order_ms:>9.2}");
        }
        println!(" |");
    }
    Ok(())
}

fn median(runs: impl Iterator<Item = f64>) -> f64 {
    let mut values: Vec<f64> = runs.collect();
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}

fn ms(started: Instant) -> f64 {
    started.elapsed().as_secs_f64() * 1000.0
}
