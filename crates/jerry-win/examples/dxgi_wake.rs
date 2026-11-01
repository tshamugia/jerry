//! Spike #11: how long a new process's first DXGI calls take.
//!
//! WebView2's GPU process calls Chromium's `CollectDriverInfoD3D` before its first frame: a DXGI
//! factory, then every adapter's description and driver version. On a hybrid laptop (Intel +
//! NVIDIA) this can take more than a second. This runs the same calls in a fresh process after
//! each pause, so every measurement is a process's first DXGI use, like the GPU process's.
//!
//! ```text
//! cargo run -p jerry-win --release --example dxgi_wake -- [pauses in seconds, default 0,5,15,30,60]
//! ```

use std::process::Command;
use std::time::{Duration, Instant};

use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIDevice, IDXGIFactory1};
use windows::core::Interface;

fn ms(started: Instant) -> f64 {
    started.elapsed().as_secs_f64() * 1000.0
}

/// One process's first DXGI calls, timed step by step.
fn measure() -> windows::core::Result<String> {
    let started = Instant::now();
    // SAFETY: plain COM factory creation.
    let factory: IDXGIFactory1 = unsafe { CreateDXGIFactory1()? };
    let mut line = format!("factory {:.0} ms", ms(started));
    let mut index = 0;
    loop {
        let step = Instant::now();
        // SAFETY: EnumAdapters1 fails with DXGI_ERROR_NOT_FOUND after the last adapter.
        let Ok(adapter) = (unsafe { factory.EnumAdapters1(index) }) else {
            break;
        };
        // SAFETY: `adapter` is a valid adapter from the factory.
        let desc = unsafe { adapter.GetDesc1()? };
        let enumerated = ms(step);
        let len = desc.Description.iter().position(|&c| c == 0).unwrap_or(0);
        let name = String::from_utf16_lossy(&desc.Description[..len]);
        let query = Instant::now();
        // SAFETY: as above; the GUID is a static IID.
        let _ = unsafe { adapter.CheckInterfaceSupport(&IDXGIDevice::IID) };
        line.push_str(&format!(
            " | {name}: enum {enumerated:.0} ms, version {:.0} ms",
            ms(query)
        ));
        index += 1;
    }
    line.push_str(&format!(" | total {:.0} ms", ms(started)));
    Ok(line)
}

#[allow(clippy::print_stdout)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let first = args.next();
    if first.as_deref() == Some("child") {
        println!("{}", measure()?);
        return Ok(());
    }
    let pauses: Vec<u64> = first
        .unwrap_or_else(|| "0,5,15,30,60".into())
        .split(',')
        .filter_map(|v| v.parse().ok())
        .collect();
    let exe = std::env::current_exe()?;
    for pause in pauses {
        std::thread::sleep(Duration::from_secs(pause));
        let output = Command::new(&exe).arg("child").output()?;
        print!(
            "after {pause:>3} s idle: {}",
            String::from_utf8_lossy(&output.stdout)
        );
    }
    Ok(())
}
