//! What a job does when a name it writes is taken (#51, PRD «კონფლიქტები»): what the user can
//! choose, the job's standing answers ("Apply to all"), the name Keep both gives, and the Recycle
//! Bin a replaced file goes to.
//!
//! A conflict stops only its own job: the engine reports it ([`Event::Conflict`]) and waits for
//! [`Control::answer`] or a cancel. What can be chosen depends on what meets what:
//!
//! | Coming | Already there | Choices |
//! | --- | --- | --- |
//! | a file or link | a file or a link to one | Replace, Skip, Keep both, Replace if newer |
//! | a file or link | a folder or a link to one | Skip, Keep both |
//! | a folder | a folder | Merge, Keep both |
//! | a folder | anything else | Keep both |
//! | anything | itself (copied into its own folder) | as with a folder there |
//!
//! An answer with "Apply to all" stays in the job's [`Policy`]: one for files, one for folders.
//! Folders inside a folder being merged merge too, as in Explorer; the files in them are asked
//! about. Replace never overwrites (data safety rule 3): the copy is written in full under its
//! temporary name, then the file it replaces goes to the Recycle Bin ([`Bin`]), and only then the
//! copy takes the name. A file the Recycle Bin can't take (network and removable drives, long
//! paths) is asked about again, and Replace then deletes it for good (data safety rule 5).
//!
//! [`Event::Conflict`]: super::Event::Conflict
//! [`Control::answer`]: super::Control::answer

use std::ffi::OsString;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::delete::{Item, is_within, probe};
use crate::error::{ErrorCode, JerryError};

/// What the user can do about a name that is taken.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
#[serde(rename = "ConflictChoice")]
pub enum Choice {
    /// The copy takes the name; what was there goes to the Recycle Bin.
    Replace,
    /// This item isn't copied or moved; what is there stays.
    Skip,
    /// Both stay: the copy gets a free name, `name (2).ext`.
    KeepBoth,
    /// Replace if the coming file was modified later than the one there, else Skip.
    ReplaceIfNewer,
    /// A folder's content goes into the folder that is there.
    Merge,
}

use Choice::{KeepBoth, Merge, Replace, ReplaceIfNewer, Skip};

/// A file onto a file.
pub const FILE_CHOICES: &[Choice] = &[Replace, Skip, KeepBoth, ReplaceIfNewer];
/// A file onto a folder.
pub const ONTO_FOLDER_CHOICES: &[Choice] = &[Skip, KeepBoth];
/// A folder onto a folder.
pub const FOLDER_CHOICES: &[Choice] = &[Merge, KeepBoth];
/// A folder onto anything else, or onto itself.
pub const ONTO_FILE_CHOICES: &[Choice] = &[KeepBoth];
/// Replace was chosen, and the file there can't go to the Recycle Bin.
pub const NO_BIN_CHOICES: &[Choice] = &[Replace, Skip, KeepBoth];

/// The user's answer to a conflict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
#[serde(rename = "ConflictAnswer")]
pub struct Answer {
    pub choice: Choice,
    /// "Apply to all": the same answer for every later conflict of this kind in the job.
    pub all: bool,
}

/// One side of a conflict, as the window shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Side {
    /// A folder, not a file or a link.
    pub folder: bool,
    pub attributes: u32,
    pub size: u64,
    /// When it was last modified (`FILETIME`), 0 if that can't be read.
    pub modified: u64,
}

/// A name the job writes is taken.
#[derive(Debug, Clone, Copy)]
pub struct Conflict<'a> {
    /// What comes.
    pub source: &'a Path,
    /// Where it goes, and what is there.
    pub target: &'a Path,
    pub incoming: Side,
    pub existing: Side,
    /// What can be chosen, in the order to show.
    pub choices: &'static [Choice],
    /// Replace was chosen, and the file there can't go to the Recycle Bin: Replace now deletes it
    /// for good.
    pub no_recycle_bin: bool,
}

/// Where a file that Replace replaces goes: Windows' Recycle Bin, which `jerry_win` sends it to
/// (core doesn't call the Shell, CLAUDE.md), or a fake in tests.
pub trait Bin: Send + Sync {
    /// Sends the file or link at `path` to the Recycle Bin. Fails with
    /// [`ErrorCode::NoRecycleBin`], leaving it where it is, where none can take it.
    fn recycle(&self, path: &Path) -> Result<(), JerryError>;
}

impl<F: Fn(&Path) -> Result<(), JerryError> + Send + Sync> Bin for F {
    fn recycle(&self, path: &Path) -> Result<(), JerryError> {
        self(path)
    }
}

/// A job's standing answers: what "Apply to all" chose. It stays with the job for its retries,
/// but not across a Jerry that stopped: a resumed job asks again (Tech PRD «Crash recovery»).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct Policy {
    /// The answer to every conflict of a file or link.
    files: Option<Choice>,
    /// The answer to every conflict of a folder.
    folders: Option<Choice>,
    /// A replaced file the Recycle Bin can't take is deleted for good without asking.
    for_good: bool,
}

impl Policy {
    /// The standing answer to a conflict of an item that is a folder or not (`folder`) with
    /// `choices`, if there is one that applies.
    pub(super) fn answer(&self, folder: bool, choices: &[Choice], no_bin: bool) -> Option<Choice> {
        if no_bin {
            return self.for_good.then_some(Replace);
        }
        let standing = if folder { self.folders } else { self.files };
        standing.filter(|choice| choices.contains(choice))
    }

    /// Takes in the user's answer: kept if it applies to all.
    pub(super) fn learn(&mut self, folder: bool, no_bin: bool, answer: Answer) {
        if !answer.all {
            return;
        }
        match (no_bin, folder) {
            (true, _) if answer.choice == Replace => self.for_good = true,
            (_, false) => self.files = Some(answer.choice),
            (_, true) => self.folders = Some(answer.choice),
        }
    }
}

/// What can be chosen when `incoming` meets `existing` at `target`.
pub(super) fn choices(incoming: &Item, existing: &Item, target: &Path) -> &'static [Choice] {
    let itself = same_place(&incoming.path, target);
    let existing_folder = existing.is_dir();
    if incoming.is_dir() && !incoming.link {
        if existing_folder && !existing.link && !itself {
            FOLDER_CHOICES
        } else {
            ONTO_FILE_CHOICES
        }
    } else if existing_folder || itself {
        ONTO_FOLDER_CHOICES
    } else {
        FILE_CHOICES
    }
}

/// Whether two paths name the same place, as Windows compares names.
fn same_place(a: &Path, b: &Path) -> bool {
    is_within(a, b) && is_within(b, a)
}

/// The longest name a file system takes, in UTF-16 units.
const MAX_NAME: usize = 255;

/// How many numbered names Keep both tries before it gives up.
const KEPT_NAMES: u32 = 10_000;

/// The first free name Keep both can give the item going to `target`: `name (2).ext`, or
/// `folder (2)` for a folder. A name that ends in a number in brackets already counts on from it:
/// `name (2).ext` becomes `name (3).ext`. Fails with [`ErrorCode::InvalidName`] if none is free.
pub(super) fn free_name(target: &Path, folder: bool) -> Result<PathBuf, JerryError> {
    kept_names(target, folder)
        .find(|candidate| probe(candidate).is_err_and(|error| error.code == ErrorCode::NotFound))
        .ok_or_else(|| JerryError::new(ErrorCode::InvalidName).at(target))
}

/// The names Keep both tries for `target`, in order.
pub(super) fn kept_names(target: &Path, folder: bool) -> impl Iterator<Item = PathBuf> + '_ {
    let name: Vec<u16> = target
        .file_name()
        .unwrap_or_default()
        .encode_wide()
        .collect();
    let (stem, extension) = if folder {
        (name.as_slice(), &[][..])
    } else {
        split_extension(&name)
    };
    let (base, first) = numbered(stem);
    let (base, extension) = (base.to_vec(), extension.to_vec());
    (first..first.saturating_add(KEPT_NAMES)).map(move |number| {
        let suffix: Vec<u16> = format!(" ({number})").encode_utf16().collect();
        let room = MAX_NAME.saturating_sub(suffix.len() + extension.len());
        let mut keep = base.len().min(room);
        // Not between the two halves of a surrogate pair.
        if base
            .get(keep)
            .is_some_and(|unit| (0xDC00..=0xDFFF).contains(unit))
        {
            keep = keep.saturating_sub(1);
        }
        let mut kept = base[..keep].to_vec();
        kept.extend_from_slice(&suffix);
        kept.extend_from_slice(&extension);
        target.with_file_name(OsString::from_wide(&kept))
    })
}

/// A file name's stem and extension (with its dot): after the last dot, unless that dot starts
/// the name or the rest has a space. `archive.tar.gz` is `archive.tar` and `.gz`; `.gitignore`
/// has none.
fn split_extension(name: &[u16]) -> (&[u16], &[u16]) {
    const DOT: u16 = b'.' as u16;
    const SPACE: u16 = b' ' as u16;
    match name.iter().rposition(|&unit| unit == DOT) {
        Some(dot) if dot > 0 && dot + 1 < name.len() && !name[dot + 1..].contains(&SPACE) => {
            name.split_at(dot)
        }
        _ => (name, &[]),
    }
}

/// A stem without the number in brackets it ends with, and the number to go on from: `a (2)` is
/// `a` and 3, `a` is `a` and 2.
fn numbered(stem: &[u16]) -> (&[u16], u32) {
    let text = String::from_utf16_lossy(stem);
    let parsed = text
        .strip_suffix(')')
        .and_then(|rest| rest.rsplit_once(" ("))
        .filter(|(_, digits)| !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
        .and_then(|(base, digits)| {
            Some((base.encode_utf16().count(), digits.parse::<u32>().ok()?))
        });
    match parsed {
        // Only where the lossy text maps one to one onto the stem.
        Some((base, number)) if number >= 1 && !text.contains('\u{FFFD}') => {
            (&stem[..base], number.saturating_add(1))
        }
        _ => (stem, 2),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(target: &str, folder: bool, count: usize) -> Vec<String> {
        kept_names(Path::new(target), folder)
            .take(count)
            .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn keep_both_numbers_the_name_before_its_extension() {
        assert_eq!(
            names(r"\\?\C:\a\ნახაზი.dwg", false, 2),
            ["ნახაზი (2).dwg", "ნახაზი (3).dwg"]
        );
        assert_eq!(
            names(r"\\?\C:\a\archive.tar.gz", false, 1),
            ["archive.tar (2).gz"]
        );
        assert_eq!(names(r"\\?\C:\a\.gitignore", false, 1), [".gitignore (2)"]);
        assert_eq!(names(r"\\?\C:\a\README", false, 1), ["README (2)"]);
        assert_eq!(names(r"\\?\C:\a\v1.2 final", false, 1), ["v1.2 final (2)"]);
    }

    #[test]
    fn a_folder_keeps_its_dots() {
        assert_eq!(names(r"\\?\C:\a\v1.2", true, 1), ["v1.2 (2)"]);
        assert_eq!(names(r"\\?\C:\a\პროექტი", true, 1), ["პროექტი (2)"]);
    }

    #[test]
    fn a_numbered_name_counts_on() {
        assert_eq!(
            names(r"\\?\C:\a\a (2).txt", false, 2),
            ["a (3).txt", "a (4).txt"]
        );
        assert_eq!(names(r"\\?\C:\a\a (0).txt", false, 1), ["a (0) (2).txt"]);
        assert_eq!(names(r"\\?\C:\a\a ().txt", false, 1), ["a () (2).txt"]);
        assert_eq!(names(r"\\?\C:\a\(2)", true, 1), ["(2) (2)"]);
    }

    #[test]
    fn a_long_name_makes_room_for_its_number() {
        let name = format!("{}.dwg", "ა".repeat(260));
        let kept = names(&format!(r"\\?\C:\a\{name}"), false, 1).remove(0);
        assert_eq!(kept.encode_utf16().count(), MAX_NAME);
        assert!(kept.ends_with(" (2).dwg"));
    }

    #[test]
    fn apply_to_all_keeps_one_answer_for_files_and_one_for_folders() {
        let mut policy = Policy::default();
        assert_eq!(policy.answer(false, FILE_CHOICES, false), None);
        policy.learn(
            false,
            false,
            Answer {
                choice: Skip,
                all: false,
            },
        );
        assert_eq!(
            policy.answer(false, FILE_CHOICES, false),
            None,
            "only this once"
        );

        policy.learn(
            false,
            false,
            Answer {
                choice: Replace,
                all: true,
            },
        );
        policy.learn(
            true,
            false,
            Answer {
                choice: Merge,
                all: true,
            },
        );
        assert_eq!(policy.answer(false, FILE_CHOICES, false), Some(Replace));
        assert_eq!(policy.answer(true, FOLDER_CHOICES, false), Some(Merge));
        assert_eq!(
            policy.answer(false, ONTO_FOLDER_CHOICES, false),
            None,
            "Replace doesn't apply to a file onto a folder"
        );
        assert_eq!(policy.answer(true, ONTO_FILE_CHOICES, false), None);
        assert_eq!(
            policy.answer(false, NO_BIN_CHOICES, true),
            None,
            "deleting for good is asked about on its own"
        );
        policy.learn(
            false,
            true,
            Answer {
                choice: Replace,
                all: true,
            },
        );
        assert_eq!(policy.answer(false, NO_BIN_CHOICES, true), Some(Replace));
        assert_eq!(policy.answer(false, FILE_CHOICES, false), Some(Replace));
    }

    #[test]
    fn skipping_all_where_there_is_no_recycle_bin_skips_every_file() {
        let mut policy = Policy::default();
        policy.learn(
            false,
            true,
            Answer {
                choice: Skip,
                all: true,
            },
        );
        assert_eq!(policy.answer(false, FILE_CHOICES, false), Some(Skip));
        assert_eq!(policy.answer(false, NO_BIN_CHOICES, true), None);
    }
}
