//! Spike #18: a folder of Georgian and otherwise unusual names, to check rendering, natural sort
//! and filtering. Always `%TEMP%\jerry-synth\ქართული სახელები`, never a real user folder.

use std::ffi::OsString;
use std::fs::File;
use std::io;
use std::os::windows::ffi::OsStringExt;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use crate::listing::long_path;

/// The folder's own name is Georgian too, so the breadcrumb and the path round trip get tested.
pub const DIR_NAME: &str = "ქართული სახელები";

/// What a name is there to test, for the spike's readout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Plain Mkhedruli, the everyday Georgian script.
    Mkhedruli,
    /// Georgian mixed with Latin, digits and punctuation, as in real project folders.
    Mixed,
    /// Numbered Georgian names for natural sort.
    Numbered,
    /// Long enough to be cut with an ellipsis everywhere.
    Long,
    /// Mtavruli (U+1C90..), the uppercase Georgian added in Unicode 11.
    Mtavruli,
    /// Asomtavruli and Nuskhuri (the church scripts) and archaic Mkhedruli letters.
    Historic,
    /// Emoji, combining marks, RTL, CJK, invisible characters and full-width digits.
    Unusual,
}

/// The names (UTF-8) with what each one tests; `true` makes a folder.
pub const NAMES: &[(&str, Kind, bool)] = &[
    ("ნახაზები", Kind::Mkhedruli, true),
    ("ფოტოები და ვიდეო", Kind::Mkhedruli, true),
    ("ხელშეკრულება.pdf", Kind::Mkhedruli, false),
    ("ანგარიში.docx", Kind::Mkhedruli, false),
    ("ა.txt", Kind::Mkhedruli, false),
    ("ბ.txt", Kind::Mkhedruli, false),
    ("ზ.txt", Kind::Mkhedruli, false),
    ("ჰ.txt", Kind::Mkhedruli, false),
    ("ფქჭღყ ჯჰძწ.txt", Kind::Mkhedruli, false),
    ("პროექტი 2", Kind::Numbered, true),
    ("პროექტი 10", Kind::Numbered, true),
    ("ნახაზი 1.dwg", Kind::Numbered, false),
    ("ნახაზი 2.dwg", Kind::Numbered, false),
    ("ნახაზი 9.dwg", Kind::Numbered, false),
    ("ნახაზი 10.dwg", Kind::Numbered, false),
    ("ნახაზი 11.dwg", Kind::Numbered, false),
    ("ნახაზი 100.dwg", Kind::Numbered, false),
    ("ნახაზი 007.dwg", Kind::Numbered, false),
    ("სართული 1 გეგმა v2.dwg", Kind::Numbered, false),
    ("სართული 1 გეგმა v10.dwg", Kind::Numbered, false),
    ("სართული 12 ჭრილი.dwg", Kind::Numbered, false),
    ("ბათუმი 2026", Kind::Mixed, true),
    ("Project ბათუმი Tower B-3 rev.4.dwg", Kind::Mixed, false),
    ("INV-2025-014 ინვოისი TSGroup.pdf", Kind::Mixed, false),
    ("CCTV კამერები 64ch NVR (საბოლოო).xlsx", Kind::Mixed, false),
    ("„ციტატა“ – №5, 50% … ¶.txt", Kind::Mixed, false),
    ("ანგარიში.ფაილი", Kind::Mixed, false),
    (
        "ტექნიკური დავალება — ბათუმის ბულვარის მრავალფუნქციური კომპლექსის სახანძრო სიგნალიზაციის, \
         ვიდეოთვალთვალისა და დაშვების კონტროლის სისტემების პროექტი, საბოლოო ვერსია \
         (შეთანხმებულია 14.10.2026).pdf",
        Kind::Long,
        false,
    ),
    (
        "ძალიან_გრძელი_სახელი_ჰარების_გარეშე_რომელიც_სვეტში_არ_ეტევა_და_ბოლოში_სამი_წერტილით_უნდა_მოიჭრას.dwg",
        Kind::Long,
        false,
    ),
    ("ᲜᲐᲮᲐᲖᲘ ᲡᲐᲑᲝᲚᲝᲝ.dwg", Kind::Mtavruli, false),
    ("ႠႡႢ ⴀⴁⴂ ხუცური.txt", Kind::Historic, false),
    ("ჱ ჲ ჳ ჴ ჵ ჶ ჷ ჸ ჹ ჺ ჻ ჼ ჽ ჾ ჿ.txt", Kind::Historic, false),
    ("🇬🇪 საქართველო 📁 არქივი 👨‍👩‍👧.zip", Kind::Unusual, false),
    ("ა̈ ე́ ო̃ combining.txt", Kind::Unusual, false),
    ("مرحبا שלום ქართული.txt", Kind::Unusual, false),
    ("日本語 中文 한국어 ნახაზი.txt", Kind::Unusual, false),
    ("ორი\u{00A0}სიტყვა\u{200B}უხილავი.txt", Kind::Unusual, false),
    ("ნახაზი ３.dwg", Kind::Unusual, false),
    ("Ångström ǅ ß ẞ ﬁ.txt", Kind::Unusual, false),
];

/// A name that isn't valid Unicode (an unpaired surrogate). NTFS allows it; Jerry must list it,
/// show it with U+FFFD and still operate on the exact name.
pub fn unpaired_surrogate_name() -> OsString {
    let mut units: Vec<u16> = "უცნაური ".encode_utf16().collect();
    units.push(0xD800);
    units.extend(" სახელი.txt".encode_utf16());
    OsString::from_wide(&units)
}

/// `%TEMP%\jerry-synth\ქართული სახელები`.
pub fn dir() -> PathBuf {
    super::root().join(DIR_NAME)
}

/// Every name the folder holds, with the unpaired surrogate one last.
pub fn names() -> Vec<OsString> {
    let mut names: Vec<OsString> = NAMES.iter().map(|(name, ..)| (*name).into()).collect();
    names.push(unpaired_surrogate_name());
    names
}

/// Creates the folder and whatever entries it misses (idempotent, safe to race). Returns its path.
pub fn ensure() -> io::Result<PathBuf> {
    let dir = dir();
    std::fs::create_dir_all(long_path(&dir))?;
    let modified = SystemTime::UNIX_EPOCH + Duration::from_secs(1_793_000_000);
    for (index, (name, _, is_dir)) in NAMES.iter().enumerate() {
        let path = long_path(&dir.join(name));
        let created = if *is_dir {
            std::fs::create_dir(&path)
        } else {
            File::create_new(&path).and_then(|file| {
                file.set_len(index as u64 * 1_337)?;
                file.set_modified(modified + Duration::from_secs(index as u64 * 3_600))
            })
        };
        exists_ok(created)?;
    }
    exists_ok(File::create_new(long_path(&dir.join(unpaired_surrogate_name()))).map(drop))?;
    Ok(dir)
}

fn exists_ok(result: io::Result<()>) -> io::Result<()> {
    match result {
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Ok(()),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    use crate::listing::list_dir;
    use std::sync::atomic::AtomicBool;

    #[test]
    fn names_are_unique_and_fit_a_path_component() {
        let unique: HashSet<&str> = NAMES.iter().map(|(name, ..)| *name).collect();
        assert_eq!(unique.len(), NAMES.len());
        for (name, ..) in NAMES {
            let units = name.encode_utf16().count();
            assert!(units <= 255, "{name}: {units} UTF-16 units");
        }
    }

    /// The folder lists back with every name exactly as written, the unpaired surrogate included.
    #[test]
    fn listing_round_trips_every_name() {
        let dir = ensure().unwrap();
        let entries = list_dir(&dir, &AtomicBool::new(false), |_, _| {}).unwrap();
        let listed: HashSet<OsString> = (0..entries.len()).map(|id| entries.name(id)).collect();
        for name in names() {
            assert!(listed.contains(&name), "missing {name:?}");
        }
    }

    /// Unicode 11 made Mtavruli the uppercase of Mkhedruli (JS `toUpperCase` maps them), but
    /// NTFS's case table doesn't: both names live side by side in one folder. So name conflicts
    /// must come from the file system, never from a JS case fold.
    #[test]
    fn ntfs_keeps_mtavruli_and_mkhedruli_apart() {
        let dir = super::super::root().join(format!("case-probe-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        File::create_new(dir.join("ნახაზი.txt")).unwrap();
        let second = File::create_new(dir.join("ᲜᲐᲮᲐᲖᲘ.txt"));
        // Latin for comparison: NTFS folds case, so this one collides.
        File::create_new(dir.join("drawing.txt")).unwrap();
        let latin = File::create_new(dir.join("DRAWING.txt"));
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(second.is_ok(), "Mtavruli collided: {second:?}");
        assert_eq!(
            latin.map(drop).unwrap_err().kind(),
            io::ErrorKind::AlreadyExists
        );
    }
}
