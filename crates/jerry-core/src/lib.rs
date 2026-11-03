//! Jerry's file core: listing, sorting, watcher, transfers, undo, search, rename, ZIP and storage.
//!
//! Plain Win32 file APIs only. No COM, no Tauri, no UI: everything here is testable on temp
//! folders. Shell capabilities come in through traits that `jerry-win` implements.

pub mod error;
pub mod job;
pub mod listing;
pub mod path;
pub mod settings;
pub mod sort;
pub mod storage;
pub mod synth;

pub use error::{ErrorCode, JerryError};

/// Version of the core crate (the workspace version).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_matches_workspace() {
        assert_eq!(VERSION, env!("CARGO_PKG_VERSION"));
        assert!(!VERSION.is_empty());
    }
}
