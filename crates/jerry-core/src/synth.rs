//! Synthetic folders for measuring and benchmarking: `count` entries with mixed extensions,
//! numbers for natural sort, Georgian names and about 2 % subfolders. Always under the user's
//! temp folder (`%TEMP%\jerry-synth\<count>`), never in a real user folder.
//!
//! The content is deterministic: the same `count` always gives the same names, sizes and dates.
//! `cargo run -p jerry-core --release --example synth` builds the standard sizes.

use std::fs::{File, OpenOptions};
use std::io;
use std::num::NonZero;
use std::os::windows::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// The standard sizes: the PRD's budgets and the benchmarks (#44).
pub const SIZES: [usize; 3] = [1_000, 10_000, 100_000];

/// `FILE_WRITE_ATTRIBUTES`: enough to set a folder's modified time.
const FILE_WRITE_ATTRIBUTES: u32 = 0x100;
/// `FILE_FLAG_BACKUP_SEMANTICS`: needed to open a folder as a handle.
const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;

/// One generated entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SynthEntry {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    pub modified: SystemTime,
}

/// `%TEMP%\jerry-synth`: every synthetic folder lives here.
pub fn root() -> PathBuf {
    std::env::temp_dir().join("jerry-synth")
}

/// The folder for `count` entries.
pub fn folder(count: usize) -> PathBuf {
    root().join(count.to_string())
}

/// Written once a folder is complete.
fn done_marker(count: usize) -> PathBuf {
    root().join(format!("{count}.done"))
}

/// The entries of a folder with `count` items (deterministic).
pub fn entries(count: usize) -> Vec<SynthEntry> {
    (1..=count as u64).map(entry).collect()
}

fn entry(index: u64) -> SynthEntry {
    let mut rng = SplitMix64(index.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0x004A_6572_7279);
    let i = index;
    let is_dir = rng.below(100) < 2;
    let name = if is_dir {
        match rng.below(3) {
            0 => format!("Folder {i}"),
            1 => format!("პროექტი {i}"),
            _ => format!("Archive {i}"),
        }
    } else {
        match rng.below(20) {
            0 => format!("IMG_{i:04}.jpg"),
            1 => format!("DSC{i:05}.JPG"),
            2 => format!("Report {i}.docx"),
            3 => format!("report {i} final.pdf"),
            4 => format!("file{i}.txt"),
            5 => format!("ნახაზი {i}.dwg"),
            6 => format!("სართული {} გეგმა {i}.dwg", i % 20 + 1),
            7 => format!("ხელშეკრულება №{i}.pdf"),
            8 => format!("ინვოისი {i}.xlsx"),
            9 => format!("ფოტო {i}.png"),
            10 => format!(
                "Invoice-2025-{:02}-{:02}-{i}.pdf",
                rng.below(12) + 1,
                rng.below(28) + 1
            ),
            11 => format!("Project_v{}.{}.{}.zip", i / 10_000, i / 100 % 100, i % 100),
            12 => format!("Readme{i}.MD"),
            13 => format!("data_{i}.csv"),
            14 => format!("Scan {i}.tif"),
            15 => format!("README {i}"),
            16 => format!(".config-{i}"),
            17 => format!("ანგარიში {i} (ასლი).docx"),
            18 => format!("Plan {i}A.dwg"),
            _ => format!("video-{i}.mp4"),
        }
    };
    // Mostly small enough to live in the MFT record, so 100 000 files take little disk space.
    let size = match (is_dir, rng.below(100)) {
        (true, _) => 0,
        (false, 0..95) => rng.below(700),
        (false, _) => 700 + rng.below(32_000),
    };
    // Spread over ten years from 2016-01-01.
    let modified =
        SystemTime::UNIX_EPOCH + Duration::from_secs(1_451_606_400 + rng.below(10 * 365 * 86_400));
    SynthEntry {
        name,
        is_dir,
        size,
        modified,
    }
}

/// Makes sure the folder for `count` exists and is complete, building it if not. Returns its
/// path. Building 100 000 entries takes a while; call it on a background thread.
pub fn ensure(count: usize) -> io::Result<PathBuf> {
    let dir = folder(count);
    if done_marker(count).is_file() && dir.is_dir() {
        return Ok(dir);
    }
    if dir.exists() {
        // An interrupted earlier run: only ever our own folder under %TEMP%\jerry-synth.
        std::fs::remove_dir_all(&dir)?;
    }
    std::fs::create_dir_all(&dir)?;
    let list = entries(count);
    let threads = std::thread::available_parallelism().map_or(4, NonZero::get);
    let chunk = list.len().div_ceil(threads).max(1);
    std::thread::scope(|scope| {
        let workers: Vec<_> = list
            .chunks(chunk)
            .map(|part| scope.spawn(|| part.iter().try_for_each(|entry| create(&dir, entry))))
            .collect();
        workers.into_iter().try_for_each(|worker| {
            worker
                .join()
                .unwrap_or_else(|_| Err(io::Error::other("a synth worker panicked")))
        })
    })?;
    File::create(done_marker(count))?;
    Ok(dir)
}

fn create(dir: &Path, entry: &SynthEntry) -> io::Result<()> {
    let path = dir.join(&entry.name);
    if entry.is_dir {
        std::fs::create_dir(&path)?;
        OpenOptions::new()
            .access_mode(FILE_WRITE_ATTRIBUTES)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
            .open(&path)?
            .set_modified(entry.modified)
    } else {
        let file = File::create_new(&path)?;
        if entry.size > 0 {
            file.set_len(entry.size)?;
        }
        file.set_modified(entry.modified)
    }
}

/// Deletes every synthetic folder (only `%TEMP%\jerry-synth`).
pub fn remove_all() -> io::Result<()> {
    match std::fs::remove_dir_all(root()) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}

/// A tiny deterministic PRNG (SplitMix64), so the generator needs no dependency.
struct SplitMix64(u64);

impl SplitMix64 {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, bound: u64) -> u64 {
        self.next() % bound
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn names_are_unique_case_insensitively_and_deterministic() {
        let list = entries(100_000);
        let unique: HashSet<String> = list.iter().map(|e| e.name.to_lowercase()).collect();
        assert_eq!(unique.len(), list.len());
        assert_eq!(entries(1_000), list[..1_000]);
    }

    #[test]
    fn mix_has_folders_georgian_and_numbers() {
        let list = entries(10_000);
        let folders = list.iter().filter(|e| e.is_dir).count();
        assert!((100..400).contains(&folders), "{folders} folders");
        let georgian = list
            .iter()
            .filter(|e| {
                e.name
                    .chars()
                    .any(|c| ('\u{10D0}'..='\u{10FF}').contains(&c))
            })
            .count();
        assert!(georgian > 2_000, "{georgian} Georgian names");
        assert!(
            list.iter()
                .all(|e| e.name.chars().any(|c| c.is_ascii_digit()))
        );
    }

    #[test]
    fn folders_live_under_temp() {
        assert!(folder(1_000).starts_with(std::env::temp_dir()));
        assert_eq!(folder(1_000).file_name(), Some("1000".as_ref()));
    }
}
