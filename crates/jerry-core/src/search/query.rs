//! What a search looks for (PRD «ღრმა ძებნა»): [`SearchQuery`] as the UI sends it, and
//! [`Matcher`], the query compiled once and asked about every entry the walker reads. Only what
//! the directory read gives is asked about (name, attributes, size, last write time), so no file
//! is ever opened: a OneDrive file that lives only in the cloud stays there.

use serde::Deserialize;
use specta::Type;
use windows::Win32::Storage::FileSystem::FILE_ATTRIBUTE_DIRECTORY;

use crate::sort::extension;

/// FILETIME of the Unix epoch: 100 ns ticks from 1601-01-01 to 1970-01-01.
const UNIX_EPOCH_FILETIME: f64 = 116_444_736_000_000_000.0;

/// What to look for. Every filter that is set must match; with none set, everything matches.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SearchQuery {
    /// Words of the name, separated by spaces; every word must match, in any order. A word
    /// without `*` or `?` is a part of the name (`rev` matches `plan_rev2.dwg`); one with them is
    /// the whole name, as in the quick filter (`*.pdf`, `rev*`): `*` any run of characters, `?`
    /// exactly one. Case doesn't count, in any script (Mtavruli matches Mkhedruli). Empty matches
    /// every name.
    pub name: String,
    /// A type group or one extension. Only files have a type, so with it no folder matches.
    pub file_type: Option<FileType>,
    /// Size in bytes. Only files have a size here, so with it no folder matches.
    pub size: Option<SizeRange>,
    /// Last write time.
    pub modified: Option<DateRange>,
    /// Files, folders or both.
    pub items: SearchItems,
    /// Hidden files and folders too. Without it they neither match nor are searched, as the
    /// list hides them without Ctrl+H.
    pub hidden: bool,
    /// Search inside links (symbolic links, junctions, mount points) too. Off, a link can still
    /// match, but what it points to isn't searched.
    pub follow_links: bool,
    /// Search Windows' own folders too (Windows, Program Files, ProgramData), which are left out
    /// otherwise. A search that starts inside one of them always searches it.
    pub system_folders: bool,
}

/// Which kinds of items match.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum SearchItems {
    #[default]
    All,
    Files,
    Folders,
}

/// A type group, or one extension.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum FileType {
    /// Office documents, PDF, plain text.
    Documents,
    /// AutoCAD drawings: DWG and DXF.
    Drawings,
    Images,
    Archives,
    Video,
    /// Files with this extension, case ignored. A leading `*.` or `.` is dropped (`*.pdf`,
    /// `.pdf` and `pdf` are one), and empty means files without an extension.
    Extension {
        extension: String,
    },
}

/// Bytes, both ends included. A missing end is open.
#[derive(Debug, Clone, Copy, Default, PartialEq, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SizeRange {
    pub min: Option<f64>,
    pub max: Option<f64>,
}

/// Milliseconds since the Unix epoch, from `from` (included) to `to` (left out): today is
/// `[midnight, next midnight)`, in the user's time zone as the UI computes it. A missing end is
/// open.
#[derive(Debug, Clone, Copy, Default, PartialEq, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DateRange {
    pub from: Option<f64>,
    pub to: Option<f64>,
}

/// Extensions of the type groups, lower case. A group names what people open as that kind of
/// file, not every format there is.
const DOCUMENTS: &[&str] = &[
    "pdf", "doc", "docx", "docm", "dot", "dotx", "rtf", "odt", "txt", "md", "xls", "xlsx", "xlsm",
    "xlsb", "xlt", "xltx", "ods", "csv", "ppt", "pptx", "pptm", "pps", "ppsx", "odp", "xps",
    "oxps", "vsd", "vsdx",
];
const DRAWINGS: &[&str] = &["dwg", "dxf"];
const IMAGES: &[&str] = &[
    "jpg", "jpeg", "jpe", "jfif", "png", "gif", "bmp", "dib", "tif", "tiff", "webp", "heic",
    "heif", "avif", "ico", "svg", "psd", "dng", "cr2", "cr3", "nef", "arw", "orf", "rw2", "raf",
];
const ARCHIVES: &[&str] = &[
    "zip", "rar", "7z", "tar", "gz", "tgz", "bz2", "tbz2", "xz", "txz", "zst", "lz", "lzma", "cab",
];
const VIDEO: &[&str] = &[
    "mp4", "m4v", "mov", "avi", "mkv", "wmv", "webm", "mpg", "mpeg", "m2ts", "mts", "3gp", "flv",
    "vob", "ogv",
];

/// A word of the name.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Word {
    /// A part of the name, folded.
    Part(Vec<char>),
    /// The whole name with `*` and `?`, folded.
    Pattern(Vec<char>),
}

/// A query compiled for asking about entries: names and extensions folded once, sizes and dates
/// as the directory read gives them.
#[derive(Debug, Clone)]
pub(crate) struct Matcher {
    words: Vec<Word>,
    /// Folded extensions without the dot, one of which a file's must be.
    extensions: Option<Vec<Vec<char>>>,
    min_size: u64,
    max_size: u64,
    /// FILETIMEs: `from` included, `to` left out.
    from: u64,
    to: u64,
    items: SearchItems,
    /// A filter only files have is set.
    files_only: bool,
}

impl Matcher {
    pub(crate) fn new(query: &SearchQuery) -> Self {
        let extensions = query.file_type.as_ref().map(|file_type| match file_type {
            FileType::Documents => folded_list(DOCUMENTS),
            FileType::Drawings => folded_list(DRAWINGS),
            FileType::Images => folded_list(IMAGES),
            FileType::Archives => folded_list(ARCHIVES),
            FileType::Video => folded_list(VIDEO),
            FileType::Extension { extension } => {
                let extension = extension
                    .trim()
                    .trim_start_matches('*')
                    .trim_start_matches('.');
                vec![extension.chars().map(fold).collect()]
            }
        });
        let size = query.size.unwrap_or_default();
        let modified = query.modified.unwrap_or_default();
        Self {
            words: words(&query.name),
            files_only: extensions.is_some() || query.size.is_some(),
            extensions,
            min_size: size.min.map_or(0, bytes),
            max_size: size.max.map_or(u64::MAX, bytes),
            from: modified.from.map_or(0, filetime),
            to: modified.to.map_or(u64::MAX, filetime),
            items: query.items,
        }
    }

    /// Whether the entry matches. `folded` is room for the folded name, reused between calls.
    pub(crate) fn matches(
        &self,
        name: &[u16],
        attributes: u32,
        size: u64,
        modified: u64,
        folded: &mut Vec<char>,
    ) -> bool {
        let is_dir = attributes & FILE_ATTRIBUTE_DIRECTORY.0 != 0;
        let kind = match self.items {
            SearchItems::All => true,
            SearchItems::Files => !is_dir,
            SearchItems::Folders => is_dir,
        };
        if !kind || (is_dir && self.files_only) {
            return false;
        }
        if !is_dir {
            if size < self.min_size || size > self.max_size {
                return false;
            }
            if let Some(extensions) = &self.extensions {
                let own = extension(name);
                if !extensions.iter().any(|wanted| folds_to(own, wanted)) {
                    return false;
                }
            }
        }
        if modified < self.from || modified >= self.to {
            return false;
        }
        if self.words.is_empty() {
            return true;
        }
        folded.clear();
        folded.extend(
            char::decode_utf16(name.iter().copied())
                .map(|unit| fold(unit.unwrap_or(char::REPLACEMENT_CHARACTER))),
        );
        self.words.iter().all(|word| match word {
            Word::Part(part) => contains(folded, part),
            Word::Pattern(pattern) => wildcard(pattern, folded),
        })
    }
}

/// The words of `name`, folded. A word of stars only matches every name, so it is left out.
fn words(name: &str) -> Vec<Word> {
    name.split_whitespace()
        .filter(|word| word.chars().any(|c| c != '*'))
        .map(|word| {
            let folded: Vec<char> = word.chars().map(fold).collect();
            if word.contains(['*', '?']) {
                Word::Pattern(folded)
            } else {
                Word::Part(folded)
            }
        })
        .collect()
}

fn folded_list(extensions: &[&str]) -> Vec<Vec<char>> {
    extensions
        .iter()
        .map(|extension| extension.chars().map(fold).collect())
        .collect()
}

/// `c` with its case taken away: Unicode lower case, where it is one character (Mtavruli becomes
/// Mkhedruli, as the quick filter's case folding does, spike #18). One character for one, so `?`
/// still counts characters.
pub(crate) fn fold(c: char) -> char {
    if c.is_ascii() {
        return c.to_ascii_lowercase();
    }
    let mut lower = c.to_lowercase();
    match (lower.next(), lower.next()) {
        (Some(one), None) => one,
        _ => c,
    }
}

/// Whether `units` (UTF-16) folds to exactly `folded`.
fn folds_to(units: &[u16], folded: &[char]) -> bool {
    let mut wanted = folded.iter();
    for unit in char::decode_utf16(units.iter().copied()) {
        let c = fold(unit.unwrap_or(char::REPLACEMENT_CHARACTER));
        if wanted.next() != Some(&c) {
            return false;
        }
    }
    wanted.next().is_none()
}

/// Whether `part` occurs in `name`.
fn contains(name: &[char], part: &[char]) -> bool {
    part.is_empty() || name.windows(part.len()).any(|window| window == part)
}

/// Whether all of `name` matches `pattern`: `*` any run of characters (none too), `?` exactly
/// one. Linear in practice: a mismatch goes back only to the last star.
fn wildcard(pattern: &[char], name: &[char]) -> bool {
    let (mut p, mut n) = (0, 0);
    // The last star seen, and where in the name it stood for nothing yet.
    let mut star: Option<(usize, usize)> = None;
    while n < name.len() {
        match pattern.get(p) {
            Some('*') => {
                star = Some((p, n));
                p += 1;
            }
            Some(&c) if c == '?' || c == name[n] => {
                p += 1;
                n += 1;
            }
            _ => match star {
                // The star takes one more character.
                Some((at, from)) => {
                    star = Some((at, from + 1));
                    p = at + 1;
                    n = from + 1;
                }
                None => return false,
            },
        }
    }
    pattern[p..].iter().all(|&c| c == '*')
}

/// Bytes from the UI's number: never negative, whole.
fn bytes(value: f64) -> u64 {
    // `as` saturates, and NaN becomes 0.
    value.max(0.0) as u64
}

/// A FILETIME from ms since the Unix epoch.
fn filetime(ms: f64) -> u64 {
    (ms * 10_000.0 + UNIX_EPOCH_FILETIME).max(0.0) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: u32 = 0x20;
    const FOLDER: u32 = FILE_ATTRIBUTE_DIRECTORY.0;

    fn query(name: &str) -> SearchQuery {
        SearchQuery {
            name: name.to_owned(),
            ..SearchQuery::default()
        }
    }

    /// The names of `names` that `query` matches, as files of size 100 modified at FILETIME 0.
    fn matching<'a>(query: &SearchQuery, names: &[&'a str]) -> Vec<&'a str> {
        let matcher = Matcher::new(query);
        let mut folded = Vec::new();
        names
            .iter()
            .copied()
            .filter(|name| {
                let units: Vec<u16> = name.encode_utf16().collect();
                matcher.matches(&units, FILE, 100, 0, &mut folded)
            })
            .collect()
    }

    const NAMES: [&str; 11] = [
        "ELV-001 LAN Layout GF.dwg",
        "plan_rev2.dwg",
        "Rev3 notes.txt",
        "report.PDF",
        "report.pdf.bak",
        "ნახაზი 12.dwg",
        "ᲜᲐᲮᲐᲖᲘ.dwg",
        "სართული 3 გეგმა.pdf",
        "a.b",
        "ab",
        "📁 photos.zip",
    ];

    #[test]
    fn a_word_is_a_part_of_the_name_in_any_case() {
        assert_eq!(
            matching(&query("rev"), &NAMES),
            ["plan_rev2.dwg", "Rev3 notes.txt"]
        );
        assert_eq!(
            matching(&query("LAYOUT"), &NAMES),
            ["ELV-001 LAN Layout GF.dwg"]
        );
        assert_eq!(
            matching(&query(".pdf"), &NAMES),
            ["report.PDF", "report.pdf.bak", "სართული 3 გეგმა.pdf"]
        );
    }

    #[test]
    fn georgian_mtavruli_and_mkhedruli_are_one_case_pair() {
        assert_eq!(
            matching(&query("ნახაზ"), &NAMES),
            ["ნახაზი 12.dwg", "ᲜᲐᲮᲐᲖᲘ.dwg"]
        );
        assert_eq!(
            matching(&query("ᲜᲐᲮᲐᲖᲘ"), &NAMES),
            ["ნახაზი 12.dwg", "ᲜᲐᲮᲐᲖᲘ.dwg"]
        );
    }

    #[test]
    fn every_word_must_match_in_any_order() {
        assert_eq!(
            matching(&query("gf lan"), &NAMES),
            ["ELV-001 LAN Layout GF.dwg"]
        );
        assert_eq!(
            matching(&query("  გეგმა   სართული  "), &NAMES),
            ["სართული 3 გეგმა.pdf"]
        );
        assert_eq!(matching(&query("rev *.dwg"), &NAMES), ["plan_rev2.dwg"]);
        assert!(matching(&query("rev zzz"), &NAMES).is_empty());
    }

    #[test]
    fn a_word_with_wildcards_is_the_whole_name() {
        assert_eq!(
            matching(&query("*.pdf"), &NAMES),
            ["report.PDF", "სართული 3 გეგმა.pdf"]
        );
        assert_eq!(matching(&query("rev*"), &NAMES), ["Rev3 notes.txt"]);
        assert_eq!(matching(&query("*_rev*"), &NAMES), ["plan_rev2.dwg"]);
        assert_eq!(
            matching(&query("elv*gf.dwg"), &NAMES),
            ["ELV-001 LAN Layout GF.dwg"]
        );
        assert_eq!(matching(&query("r*t*.pdf"), &NAMES), ["report.PDF"]);
        assert_eq!(matching(&query("a**b"), &NAMES), ["a.b", "ab"]);
        assert_eq!(
            matching(&query("ნახ*"), &NAMES),
            ["ნახაზი 12.dwg", "ᲜᲐᲮᲐᲖᲘ.dwg"]
        );
    }

    #[test]
    fn a_question_mark_is_exactly_one_character_an_emoji_too() {
        assert_eq!(matching(&query("a?b"), &NAMES), ["a.b"]);
        assert_eq!(matching(&query("??"), &NAMES), ["ab"]);
        assert_eq!(matching(&query("?*photos.zip"), &NAMES), ["📁 photos.zip"]);
        assert_eq!(matching(&query("ᲜᲐᲮᲐᲖ?.dwg"), &NAMES), ["ᲜᲐᲮᲐᲖᲘ.dwg"]);
    }

    #[test]
    fn empty_names_and_lone_stars_match_everything() {
        assert_eq!(matching(&query(""), &NAMES), NAMES);
        assert_eq!(matching(&query("*"), &NAMES), NAMES);
        assert_eq!(matching(&query(" ** "), &NAMES), NAMES);
    }

    #[test]
    fn wildcards_match_whole_names() {
        let chars = |text: &str| text.chars().collect::<Vec<_>>();
        let check = |pattern: &str, name: &str| wildcard(&chars(pattern), &chars(name));
        assert!(check("*a*b*c*", "xaxbxcx"));
        assert!(!check("*a*b*c", "xaxbxcx"));
        assert!(check("a*", "a"));
        assert!(!check("a?", "a"));
        assert!(check("*aab", "aaaab"), "a star takes back what it gave");
        assert!(!check("", "a"));
        assert!(check("", ""));
    }

    fn typed(file_type: FileType) -> SearchQuery {
        SearchQuery {
            file_type: Some(file_type),
            ..SearchQuery::default()
        }
    }

    const TYPED: [&str; 12] = [
        "გეგმა.DWG",
        "plan.dxf",
        "plan.dwf",
        "offer.docx",
        "notes.txt",
        "photo.JPEG",
        "backup.7z",
        "clip.mkv",
        "README",
        "archive.tar.gz",
        "ფაილი.ნახაზი",
        ".gitignore",
    ];

    #[test]
    fn type_groups_go_by_extension_in_any_case() {
        assert_eq!(
            matching(&typed(FileType::Drawings), &TYPED),
            ["გეგმა.DWG", "plan.dxf"]
        );
        assert_eq!(
            matching(&typed(FileType::Documents), &TYPED),
            ["offer.docx", "notes.txt"]
        );
        assert_eq!(matching(&typed(FileType::Images), &TYPED), ["photo.JPEG"]);
        assert_eq!(
            matching(&typed(FileType::Archives), &TYPED),
            ["backup.7z", "archive.tar.gz"]
        );
        assert_eq!(matching(&typed(FileType::Video), &TYPED), ["clip.mkv"]);
    }

    #[test]
    fn one_extension_however_it_is_written() {
        let extension = |text: &str| {
            typed(FileType::Extension {
                extension: text.to_owned(),
            })
        };
        for written in ["dwf", ".DWF", "*.dwf", " dwf "] {
            assert_eq!(matching(&extension(written), &TYPED), ["plan.dwf"]);
        }
        assert_eq!(matching(&extension("ნახაზი"), &TYPED), ["ფაილი.ნახაზი"]);
        assert_eq!(matching(&extension("gitignore"), &TYPED), [".gitignore"]);
        assert_eq!(
            matching(&extension(""), &TYPED),
            ["README"],
            "empty: files without an extension"
        );
    }

    #[test]
    fn sizes_dates_and_kinds() {
        let matcher = |query: SearchQuery| Matcher::new(&query);
        let mut folded = Vec::new();
        let name: Vec<u16> = "a.txt".encode_utf16().collect();
        let mut test = |matcher: &Matcher, attributes, size, modified| {
            matcher.matches(&name, attributes, size, modified, &mut folded)
        };

        let sized = matcher(SearchQuery {
            size: Some(SizeRange {
                min: Some(10.0),
                max: Some(20.0),
            }),
            ..SearchQuery::default()
        });
        assert!(!test(&sized, FILE, 9, 0));
        assert!(test(&sized, FILE, 10, 0), "both ends included");
        assert!(test(&sized, FILE, 20, 0));
        assert!(!test(&sized, FILE, 21, 0));
        assert!(!test(&sized, FOLDER, 0, 0), "folders have no size here");

        let open = matcher(SearchQuery {
            size: Some(SizeRange {
                min: Some(1e15),
                max: None,
            }),
            ..SearchQuery::default()
        });
        assert!(test(&open, FILE, u64::MAX, 0));
        assert!(!test(&open, FILE, 1, 0));

        // 2026-01-01T00:00:00Z to 2026-01-02, as ms and as FILETIMEs.
        let (day, next) = (1_767_225_600_000.0, 1_767_312_000_000.0);
        let (start, end) = (134_116_992_000_000_000, 134_117_856_000_000_000);
        let dated = matcher(SearchQuery {
            modified: Some(DateRange {
                from: Some(day),
                to: Some(next),
            }),
            ..SearchQuery::default()
        });
        assert!(!test(&dated, FILE, 0, start - 1));
        assert!(test(&dated, FILE, 0, start), "from is included");
        assert!(test(&dated, FOLDER, 0, end - 1), "folders have dates");
        assert!(!test(&dated, FILE, 0, end), "to is left out");

        let files = matcher(SearchQuery {
            items: SearchItems::Files,
            ..SearchQuery::default()
        });
        assert!(test(&files, FILE, 0, 0));
        assert!(!test(&files, FOLDER, 0, 0));
        let folders = matcher(SearchQuery {
            items: SearchItems::Folders,
            ..SearchQuery::default()
        });
        assert!(!test(&folders, FILE, 0, 0));
        assert!(test(&folders, FOLDER, 0, 0));
        assert!(
            !test(&matcher(typed(FileType::Documents)), FOLDER, 0, 0),
            "folders have no type"
        );
    }

    #[test]
    fn folding_keeps_one_character_for_one() {
        assert_eq!(fold('Ა'), 'ა');
        assert_eq!(fold('ა'), 'ა');
        assert_eq!(fold('Q'), 'q');
        assert_eq!(fold('Ω'), 'ω');
        // Lower case is two characters here: kept as it is.
        assert_eq!(fold('İ'), 'İ');
    }

    #[test]
    fn names_that_are_not_unicode_still_match_by_their_other_characters() {
        let matcher = Matcher::new(&query("plan"));
        let mut name: Vec<u16> = "plan".encode_utf16().collect();
        name.push(0xD800);
        assert!(matcher.matches(&name, FILE, 0, 0, &mut Vec::new()));
    }
}
