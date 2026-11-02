//! Spike #12: why the in-app listing is slower than `listing_bench`. Lists the 10 000 and 100 000
//! folders the way the app does (a fresh thread after the process sat idle) with and without
//! opting the thread out of Windows power throttling (EcoQoS), next to the hot-loop numbers.
//!
//! ```text
//! cargo run -p jerry-core --release --example listing_sched
//! ```

#![allow(clippy::print_stdout, reason = "a command-line tool")]

use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use jerry_core::listing::list_dir;
use jerry_core::sort::SortIndex;
use jerry_core::synth;
use windows::Win32::System::Threading::{
    GetCurrentProcessorNumber, GetCurrentThread, SetThreadInformation,
    THREAD_POWER_THROTTLING_CURRENT_VERSION, THREAD_POWER_THROTTLING_EXECUTION_SPEED,
    THREAD_POWER_THROTTLING_STATE, ThreadPowerThrottling,
};

const RUNS: usize = 7;

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Hot,
    FreshThread,
    FreshThreadHighQos,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    for count in [10_000, 100_000] {
        let dir = synth::ensure(count)?;
        for (mode, name) in [
            (Mode::Hot, "hot loop, main thread"),
            (Mode::FreshThread, "fresh thread after 300 ms idle"),
            (Mode::FreshThreadHighQos, "same, throttling off (HighQoS)"),
        ] {
            let mut list = Vec::new();
            let mut index = Vec::new();
            let mut cpus = Vec::new();
            for _ in 0..RUNS {
                let dir = dir.clone();
                let job = move || {
                    if mode == Mode::FreshThreadHighQos {
                        high_qos();
                    }
                    // SAFETY: no preconditions.
                    let cpu = unsafe { GetCurrentProcessorNumber() };
                    let started = Instant::now();
                    let entries = list_dir(&dir, &AtomicBool::new(false), |_, _| {})?;
                    let list = started.elapsed();
                    let started = Instant::now();
                    let _ = SortIndex::build(&entries);
                    Ok::<_, jerry_core::error::JerryError>((list, started.elapsed(), cpu))
                };
                let (l, i, cpu) = if mode == Mode::Hot {
                    job()?
                } else {
                    std::thread::sleep(Duration::from_millis(300));
                    std::thread::spawn(job)
                        .join()
                        .map_err(|_| "listing thread panicked")??
                };
                list.push(l.as_secs_f64() * 1000.0);
                index.push(i.as_secs_f64() * 1000.0);
                cpus.push(cpu);
            }
            println!(
                "{count:>7}  {name:<32} list med {:>6.1} min {:>6.1} max {:>6.1} | index med {:>5.1} | cpus {cpus:?}",
                median(&list),
                min(&list),
                max(&list),
                median(&index)
            );
        }
    }
    Ok(())
}

/// Opts the current thread out of execution-speed throttling (EcoQoS).
fn high_qos() {
    let state = THREAD_POWER_THROTTLING_STATE {
        Version: THREAD_POWER_THROTTLING_CURRENT_VERSION,
        ControlMask: THREAD_POWER_THROTTLING_EXECUTION_SPEED,
        StateMask: 0,
    };
    // SAFETY: `state` is a valid THREAD_POWER_THROTTLING_STATE of the size passed.
    let result = unsafe {
        SetThreadInformation(
            GetCurrentThread(),
            ThreadPowerThrottling,
            (&raw const state).cast(),
            size_of::<THREAD_POWER_THROTTLING_STATE>() as u32,
        )
    };
    if let Err(error) = result {
        println!("SetThreadInformation failed: {error}");
    }
}

fn median(values: &[f64]) -> f64 {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    sorted[sorted.len() / 2]
}

fn min(values: &[f64]) -> f64 {
    values.iter().copied().fold(f64::INFINITY, f64::min)
}

fn max(values: &[f64]) -> f64 {
    values.iter().copied().fold(0.0, f64::max)
}
