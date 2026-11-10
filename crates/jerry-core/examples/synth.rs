//! Builds the synthetic test folders under `%TEMP%\jerry-synth`.
//!
//! ```text
//! cargo run -p jerry-core --release --example synth              # 1 000, 10 000, 100 000
//! cargo run -p jerry-core --release --example synth -- 5000      # one custom size
//! cargo run -p jerry-core --release --example synth -- --tree    # the search tree, 50 000
//! cargo run -p jerry-core --release --example synth -- --clean   # delete them all
//! ```

#![allow(clippy::print_stdout, reason = "a command-line tool")]

use std::time::Instant;

use jerry_core::synth;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|arg| arg == "--clean") {
        synth::remove_all()?;
        println!("removed {}", synth::root().display());
        return Ok(());
    }
    if args.iter().any(|arg| arg == "--tree") {
        let started = Instant::now();
        let dir = synth::ensure_tree(synth::TREE_SIZE)?;
        println!(
            "{:>7} entries  {:>8.0} ms  {}",
            synth::TREE_SIZE,
            started.elapsed().as_secs_f64() * 1000.0,
            dir.display()
        );
        return Ok(());
    }
    let counts: Vec<usize> = if args.is_empty() {
        synth::SIZES.to_vec()
    } else {
        args.iter()
            .map(|arg| arg.parse())
            .collect::<Result<_, _>>()?
    };
    for count in counts {
        let started = Instant::now();
        let dir = synth::ensure(count)?;
        println!(
            "{count:>7} entries  {:>8.0} ms  {}",
            started.elapsed().as_secs_f64() * 1000.0,
            dir.display()
        );
    }
    Ok(())
}
