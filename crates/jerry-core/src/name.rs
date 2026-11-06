//! Whether text can name a file or folder on Windows (Tech PRD «გადარქმევა»). One set of rules in
//! two places: here as the final check before anything is named, and in
//! `src/features/rename/name.ts` while the user types. Both are tested against the same table,
//! `tests/name_cases.json`, so they can't drift apart.
//!
//! The rules, checked in this order:
//! 1. Not empty.
//! 2. None of `\ / : * ? " < > |`, and no control characters (U+0000–U+001F).
//! 3. Not ending in a space or a dot: Windows drops them, so the file would get another name than
//!    the one typed, and core's `\\?\` paths would make one other programs can't open.
//! 4. Not a name Windows keeps for a device (`CON`, `NUL`, `COM1`…), with or without an extension:
//!    `NUL.txt` is `NUL` too.
//! 5. At most 255 UTF-16 units, the most a name can have on NTFS.

/// The longest name NTFS takes, in UTF-16 units.
pub const MAX_NAME_UNITS: usize = 255;

/// Characters a name can't have, besides control characters.
pub const FORBIDDEN: [char; 9] = ['\\', '/', ':', '*', '?', '"', '<', '>', '|'];

/// Names Windows keeps for devices. `COM` and `LPT` also take a superscript ¹, ² or ³.
const RESERVED: [&str; 6] = ["CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$"];
const NUMBERED: [&str; 2] = ["COM", "LPT"];
const SUPERSCRIPTS: [char; 3] = ['¹', '²', '³'];

/// Why text can't be a name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameProblem {
    Empty,
    /// It has this character, which Windows doesn't allow in names.
    Character(char),
    /// It ends in a space or a dot.
    Trailing,
    /// It is a device name, or one with an extension.
    Reserved,
    /// It is longer than [`MAX_NAME_UNITS`].
    TooLong,
}

impl NameProblem {
    /// The problem as the shared test table names it.
    pub fn kind(self) -> &'static str {
        match self {
            Self::Empty => "empty",
            Self::Character(_) => "character",
            Self::Trailing => "trailing",
            Self::Reserved => "reserved",
            Self::TooLong => "tooLong",
        }
    }
}

/// Whether `name` can name a file or folder: the first rule it breaks, if any.
pub fn check_name(name: &str) -> Result<(), NameProblem> {
    if name.is_empty() {
        return Err(NameProblem::Empty);
    }
    if let Some(forbidden) = name
        .chars()
        .find(|&c| FORBIDDEN.contains(&c) || u32::from(c) < 0x20)
    {
        return Err(NameProblem::Character(forbidden));
    }
    if name.ends_with([' ', '.']) {
        return Err(NameProblem::Trailing);
    }
    if is_reserved(name) {
        return Err(NameProblem::Reserved);
    }
    if name.encode_utf16().count() > MAX_NAME_UNITS {
        return Err(NameProblem::TooLong);
    }
    Ok(())
}

/// Whether the part before the first dot, without the spaces at its end, is a device name. Only
/// Latin letters match without regard to case, as Windows matches them.
fn is_reserved(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or(name).trim_end_matches(' ');
    if RESERVED
        .iter()
        .any(|device| stem.eq_ignore_ascii_case(device))
    {
        return true;
    }
    let mut chars = stem.chars();
    let prefix: String = chars.by_ref().take(3).collect();
    let (Some(number), None) = (chars.next(), chars.next()) else {
        return false;
    };
    NUMBERED
        .iter()
        .any(|kind| prefix.eq_ignore_ascii_case(kind))
        && (number.is_ascii_digit() || SUPERSCRIPTS.contains(&number))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_the_character_it_found() {
        assert_eq!(check_name("a:b"), Err(NameProblem::Character(':')));
        assert_eq!(check_name("tab\there"), Err(NameProblem::Character('\t')));
    }

    #[test]
    fn length_counts_utf16_units() {
        // Georgian letters are one unit each, an emoji two.
        assert_eq!(check_name(&"ა".repeat(255)), Ok(()));
        assert_eq!(check_name(&"ა".repeat(256)), Err(NameProblem::TooLong));
        assert_eq!(check_name(&"😀".repeat(127)), Ok(()));
        assert_eq!(check_name(&"😀".repeat(128)), Err(NameProblem::TooLong));
    }
}
