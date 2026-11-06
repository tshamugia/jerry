//! Natural sort in Explorer's order: `StrCmpLogicalW` semantics (file2 before file10, case
//! ignored, the user's locale), Georgian names included.
//!
//! Every name gets its sort key once per listing: Windows' own
//! `LCMapStringEx(LCMAP_SORTKEY | NORM_IGNORECASE | SORT_DIGITSASNUMBERS)`, a byte string whose
//! plain byte order is the locale's collation, the one `StrCmpLogicalW` compares by.
//! [`SortIndex::build`] makes the keys in parallel and ranks the names, extensions and types once.
//! After that [`SortIndex::order`] sorts packed integers for any column and never compares a
//! string again, so changing the sort of 100 000 entries takes a few ms.
//!
//! Dates sort by the exact `u64` FILETIME. Ties break by name, and the order is total: equal
//! keys fall back to the exact UTF-16 name, then to the entry id.

use std::collections::HashMap;
use std::time::Instant;

use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use specta::Type;
use windows::Win32::Foundation::LPARAM;
use windows::Win32::Globalization::{
    LCMAP_SORTKEY, LCMapStringEx, NORM_IGNORECASE, SORT_DIGITSASNUMBERS,
};
use windows::core::PCWSTR;

use crate::listing::Entries;

/// Flags of every sort key. A null locale name is `LOCALE_NAME_USER_DEFAULT`: the user's
/// locale, as Explorer uses.
const KEY_FLAGS: u32 = LCMAP_SORTKEY | NORM_IGNORECASE.0 | SORT_DIGITSASNUMBERS.0;

/// Names per parallel work item when building keys.
const KEY_CHUNK: usize = 2048;

/// Entries per parallel work item when packing and unpacking an order.
const ORDER_CHUNK: usize = 4096;

/// A column the list sorts by. Ordered as declared, for maps keyed by column.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Type,
)]
#[serde(rename_all = "camelCase")]
pub enum SortColumn {
    #[default]
    Name,
    /// Date modified.
    Modified,
    /// The type name, as Explorer's Type column shows it: "AutoCAD Drawing", "PDF File".
    Type,
    /// Size in bytes. Folders are 0.
    Size,
    /// Date created.
    Created,
    /// The extension: what follows the name's last dot. Folders have none.
    Extension,
}

/// How a list is sorted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Sort {
    pub column: SortColumn,
    /// Descending is ascending reversed, folders kept first.
    pub descending: bool,
    /// Folders before files, in both directions (the PRD's default; a setting).
    pub folders_first: bool,
}

impl Default for Sort {
    fn default() -> Self {
        Self {
            column: SortColumn::Name,
            descending: false,
            folders_first: true,
        }
    }
}

/// Type names for the Type column, as Explorer shows them: "File folder", "AutoCAD Drawing",
/// "PDF File". The registered names come from the Shell, so `jerry-win` implements this;
/// [`PlainTypeNames`] is core's fallback.
pub trait TypeNames {
    /// The type name of a folder (`None`), or of a file with `extension` (no dot; empty for a
    /// name without one).
    fn type_name(&self, extension: Option<&[u16]>) -> Vec<u16>;
}

/// The names Explorer gives types with nothing registered: "File folder", "PDF File" (the
/// extension's Latin letters uppercased, Georgian left as it is) and "File".
#[derive(Debug, Clone, Copy, Default)]
pub struct PlainTypeNames;

impl TypeNames for PlainTypeNames {
    fn type_name(&self, extension: Option<&[u16]>) -> Vec<u16> {
        match extension {
            None => "File folder".encode_utf16().collect(),
            Some([]) => "File".encode_utf16().collect(),
            Some(extension) => {
                let mut name: Vec<u16> = extension
                    .iter()
                    .map(|&unit| match u8::try_from(unit) {
                        Ok(byte) => u16::from(byte.to_ascii_uppercase()),
                        Err(_) => unit,
                    })
                    .collect();
                name.extend(" File".encode_utf16());
                name
            }
        }
    }
}

/// Appends the natural-sort key of `name` (UTF-16) to `out`: bytes whose plain order is
/// `StrCmpLogicalW`'s. Returns false if Windows refused, leaving `out` as it was.
pub fn append_sort_key(name: &[u16], out: &mut Vec<u8>) -> bool {
    if name.is_empty() {
        // LCMapStringEx rejects an empty source; its key is empty and sorts first.
        return true;
    }
    let mut buffer = [0u16; 512];
    let written = map_sort_key(name, &mut buffer);
    if written > 0 {
        extend_key(out, &buffer, written);
        return true;
    }
    // Too long for the stack buffer: ask for the size and try again on the heap.
    // SAFETY: `name` is a valid slice and there is no destination, so Windows only returns the
    // key's size in bytes.
    let needed =
        unsafe { LCMapStringEx(PCWSTR::null(), KEY_FLAGS, name, None, None, None, LPARAM(0)) };
    let Ok(needed) = usize::try_from(needed) else {
        return false;
    };
    let mut heap = vec![0u16; needed];
    let written = map_sort_key(name, &mut heap);
    if written == 0 {
        return false;
    }
    extend_key(out, &heap, written);
    true
}

/// Writes the sort key of `name` into `buffer`; returns its length in bytes, 0 if it failed.
fn map_sort_key(name: &[u16], buffer: &mut [u16]) -> usize {
    // SAFETY: `name` and `buffer` are valid slices. With LCMAP_SORTKEY Windows reads the
    // destination length (the slice's length in u16s) as bytes, so it writes at most half of
    // `buffer`.
    let written = unsafe {
        LCMapStringEx(
            PCWSTR::null(),
            KEY_FLAGS,
            name,
            Some(buffer),
            None,
            None,
            LPARAM(0),
        )
    };
    usize::try_from(written).unwrap_or(0)
}

/// Appends the first `len` bytes of `buffer` as they lie in memory (x64 is little-endian).
fn extend_key(out: &mut Vec<u8>, buffer: &[u16], len: usize) {
    let bytes = buffer[..len.div_ceil(2)]
        .iter()
        .flat_map(|unit| unit.to_le_bytes());
    out.extend(bytes.take(len));
}

/// The extension of `name`: what follows its last dot, empty if it has none.
pub fn extension(name: &[u16]) -> &[u16] {
    match name.iter().rposition(|&unit| unit == u16::from(b'.')) {
        Some(dot) => &name[dot + 1..],
        None => &[],
    }
}

/// Every entry's sort key, back to back in one buffer. Key `i` ends at `ends[i]`.
struct SortKeys {
    bytes: Vec<u8>,
    ends: Vec<usize>,
}

impl SortKeys {
    /// Keys for every name of `entries`, built in parallel on the rayon pool. A name Windows
    /// can't map (none is known) keys as its raw UTF-16, which still gives a stable order.
    fn build(entries: &Entries) -> Self {
        let len = entries.len();
        let parts: Vec<(Vec<u8>, Vec<usize>, usize)> = (0..len.div_ceil(KEY_CHUNK))
            .into_par_iter()
            .map(|chunk| {
                let ids = chunk * KEY_CHUNK..((chunk + 1) * KEY_CHUNK).min(len);
                let mut bytes = Vec::with_capacity(ids.len() * 64);
                let mut ends = Vec::with_capacity(ids.len());
                let mut refused = 0;
                for id in ids {
                    let name = entries.name_utf16(id);
                    if !append_sort_key(name, &mut bytes) {
                        refused += 1;
                        bytes.extend(name.iter().flat_map(|unit| unit.to_be_bytes()));
                    }
                    ends.push(bytes.len());
                }
                (bytes, ends, refused)
            })
            .collect();

        let total = parts.iter().map(|(bytes, ..)| bytes.len()).sum();
        let mut keys = Self {
            bytes: Vec::with_capacity(total),
            ends: Vec::with_capacity(len),
        };
        let mut refused = 0;
        for (bytes, ends, chunk_refused) in parts {
            let base = keys.bytes.len();
            keys.bytes.extend_from_slice(&bytes);
            keys.ends.extend(ends.into_iter().map(|end| base + end));
            refused += chunk_refused;
        }
        if refused > 0 {
            tracing::warn!(refused, "names without a sort key sort by their UTF-16");
        }
        keys
    }

    fn get(&self, id: usize) -> &[u8] {
        let start = if id == 0 { 0 } else { self.ends[id - 1] };
        &self.bytes[start..self.ends[id]]
    }
}

/// Everything needed to sort a listing's entries by any column without comparing strings
/// again. Built once per listing, and again when names change.
#[derive(Debug, Default, Clone)]
pub struct SortIndex {
    /// The indexed entry ids in ascending natural name order. An entry's position here is its
    /// name rank.
    by_name: Vec<u32>,
    /// `extension_rank[id]`: the rank of the entry's extension, from 1 (no extension) up; equal
    /// extensions (case ignored) share a rank. Folders are 0.
    extension_rank: Vec<u32>,
    /// `type_rank[id]`: the rank of the entry's type name.
    type_rank: Vec<u32>,
}

impl SortIndex {
    /// Builds the keys and ranks of all `entries`, in parallel on the rayon pool. CPU work: call
    /// it on a background thread.
    pub fn build(entries: &Entries, types: &dyn TypeNames) -> Self {
        Self::build_for(entries, (0..entries.len() as u32).collect(), types)
    }

    /// Like [`SortIndex::build`], for entries `ids` only: a live listing leaves out the entries
    /// its folder no longer has.
    pub fn build_for(entries: &Entries, mut ids: Vec<u32>, types: &dyn TypeNames) -> Self {
        let started = Instant::now();
        let keys = SortKeys::build(entries);
        let keys_ms = ms_since(started);

        ids.par_sort_unstable_by(|&a, &b| {
            let (a, b) = (a as usize, b as usize);
            keys.get(a)
                .cmp(keys.get(b))
                .then_with(|| entries.name_utf16(a).cmp(entries.name_utf16(b)))
                .then(a.cmp(&b))
        });
        let (extension_rank, type_rank) = extension_and_type_ranks(entries, types);
        tracing::debug!(
            entries = ids.len(),
            keys_ms,
            ms = ms_since(started),
            "sort index built"
        );
        Self {
            by_name: ids,
            extension_rank,
            type_rank,
        }
    }

    /// This index without entries `removed` (ascending ids): what is left keeps its ranks, so
    /// removing entries needs no new keys.
    pub fn without(&self, removed: &[u32]) -> Self {
        let mut index = self.clone();
        index
            .by_name
            .retain(|id| removed.binary_search(id).is_err());
        index
    }

    /// How many entries the index covers.
    pub fn len(&self) -> usize {
        self.by_name.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_name.is_empty()
    }

    /// The index's entry ids in display order. `entries` must be the ones the index was built
    /// from, their sizes and dates as they are now; entries added after it are left out. CPU work
    /// in parallel: a few ms for 100 000.
    pub fn order(&self, entries: &Entries, sort: Sort) -> Vec<u32> {
        // One u128 per entry: group (bit 96) | primary value (bits 32..96) | name rank (bits
        // 0..32). The rank is unique, so sorting the integers gives a total order, and the low
        // bits lead back to the entry. Descending flips value and rank but not the group.
        let flip = |value: u64, rank: u32| {
            if sort.descending {
                (!value, !rank)
            } else {
                (value, rank)
            }
        };
        let mut packed: Vec<u128> = self
            .by_name
            .par_iter()
            .enumerate()
            .with_min_len(ORDER_CHUNK)
            .map(|(rank, &id)| {
                let id = id as usize;
                let group = u128::from(!(sort.folders_first && entries.is_dir(id)));
                let value = match sort.column {
                    SortColumn::Name => 0,
                    SortColumn::Modified => entries.modified()[id],
                    SortColumn::Type => u64::from(self.type_rank[id]),
                    SortColumn::Size => entries.sizes()[id],
                    SortColumn::Created => entries.created()[id],
                    SortColumn::Extension => u64::from(self.extension_rank[id]),
                };
                let (value, rank) = flip(value, rank as u32);
                (group << 96) | (u128::from(value) << 32) | u128::from(rank)
            })
            .collect();
        packed.par_sort_unstable();
        packed
            .into_par_iter()
            .with_min_len(ORDER_CHUNK)
            .map(|key| {
                let (_, rank) = flip(0, key as u32);
                self.by_name[rank as usize]
            })
            .collect()
    }
}

/// Each entry's extension rank and type rank (see [`SortIndex`]). Keys are built once per
/// distinct extension: a folder has a few dozen.
fn extension_and_type_ranks(entries: &Entries, types: &dyn TypeNames) -> (Vec<u32>, Vec<u32>) {
    // Distinct extensions as written, without an allocation per entry. Folders are `FOLDER`.
    const FOLDER: u32 = u32::MAX;
    let mut index_of: HashMap<&[u16], u32> = HashMap::new();
    let mut extensions: Vec<&[u16]> = Vec::new();
    let extension_of: Vec<u32> = (0..entries.len())
        .map(|id| {
            if entries.is_dir(id) {
                return FOLDER;
            }
            let text = extension(entries.name_utf16(id));
            *index_of.entry(text).or_insert_with(|| {
                extensions.push(text);
                extensions.len() as u32 - 1
            })
        })
        .collect();

    let extension_keys: Vec<Vec<u8>> = extensions.iter().map(|text| key_of(text)).collect();
    let extension_ranks = dense_ranks(&extension_keys);
    // The folders' type name goes last.
    let type_keys: Vec<Vec<u8>> = extensions
        .iter()
        .map(|&text| Some(text))
        .chain([None])
        .map(|extension| key_of(&types.type_name(extension)))
        .collect();
    let type_ranks = dense_ranks(&type_keys);
    let folder_type = type_ranks[extensions.len()];

    extension_of
        .into_iter()
        .map(|index| match index {
            // Folders have no extension: 0, before every file.
            FOLDER => (0, folder_type),
            index => (extension_ranks[index as usize], type_ranks[index as usize]),
        })
        .unzip()
}

/// Dense ranks of `keys` in key order, from 1; equal keys share a rank.
fn dense_ranks(keys: &[Vec<u8>]) -> Vec<u32> {
    let mut sorted: Vec<usize> = (0..keys.len()).collect();
    sorted.sort_unstable_by(|&a, &b| keys[a].cmp(&keys[b]));
    let mut ranks = vec![0u32; keys.len()];
    let mut rank = 0;
    for (position, &i) in sorted.iter().enumerate() {
        if position == 0 || keys[sorted[position - 1]] != keys[i] {
            rank += 1;
        }
        ranks[i] = rank;
    }
    ranks
}

fn key_of(text: &[u16]) -> Vec<u8> {
    let mut key = Vec::new();
    if !append_sort_key(text, &mut key) {
        key.extend(text.iter().flat_map(|unit| unit.to_be_bytes()));
    }
    key
}

fn ms_since(started: Instant) -> f64 {
    started.elapsed().as_secs_f64() * 1000.0
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::os::windows::ffi::{OsStrExt, OsStringExt};

    use proptest::prelude::*;
    use windows::Win32::Storage::FileSystem::FILE_ATTRIBUTE_DIRECTORY;
    use windows::Win32::UI::Shell::StrCmpLogicalW;

    use super::*;

    const FOLDER: u32 = FILE_ATTRIBUTE_DIRECTORY.0;
    const FILE: u32 = 0x20;

    /// Spike #18's corpus: Georgian names in every script and form Windows meets, plus mixed,
    /// numbered, long and unusual ones. With the unpaired surrogate below, 40 names: 780 pairs.
    const GEORGIAN: &[&str] = &[
        "ნახაზები",
        "ფოტოები და ვიდეო",
        "ხელშეკრულება.pdf",
        "ანგარიში.docx",
        "ა.txt",
        "ბ.txt",
        "ზ.txt",
        "ჰ.txt",
        "ფქჭღყ ჯჰძწ.txt",
        "პროექტი 2",
        "პროექტი 10",
        "ნახაზი 1.dwg",
        "ნახაზი 2.dwg",
        "ნახაზი 9.dwg",
        "ნახაზი 10.dwg",
        "ნახაზი 11.dwg",
        "ნახაზი 100.dwg",
        "ნახაზი 007.dwg",
        "სართული 1 გეგმა v2.dwg",
        "სართული 1 გეგმა v10.dwg",
        "სართული 12 ჭრილი.dwg",
        "ბათუმი 2026",
        "Project ბათუმი Tower B-3 rev.4.dwg",
        "INV-2025-014 ინვოისი TSGroup.pdf",
        "CCTV კამერები 64ch NVR (საბოლოო).xlsx",
        "„ციტატა“ – №5, 50% … ¶.txt",
        "ანგარიში.ფაილი",
        "ტექნიკური დავალება — ბათუმის ბულვარის მრავალფუნქციური კომპლექსის სახანძრო \
         სიგნალიზაციის, ვიდეოთვალთვალისა და დაშვების კონტროლის სისტემების პროექტი, საბოლოო \
         ვერსია (შეთანხმებულია 14.10.2026).pdf",
        "ძალიან_გრძელი_სახელი_ჰარების_გარეშე_რომელიც_სვეტში_არ_ეტევა_და_ბოლოში_სამი_წერტილით_უნდა_მოიჭრას.dwg",
        "ᲜᲐᲮᲐᲖᲘ ᲡᲐᲑᲝᲚᲝᲝ.dwg",
        "ႠႡႢ ⴀⴁⴂ ხუცური.txt",
        "ჱ ჲ ჳ ჴ ჵ ჶ ჷ ჸ ჹ ჺ ჻ ჼ ჽ ჾ ჿ.txt",
        "🇬🇪 საქართველო 📁 არქივი 👨‍👩‍👧.zip",
        "ა̈ ე́ ო̃ combining.txt",
        "مرحبا שלום ქართული.txt",
        "日本語 中文 한국어 ნახაზი.txt",
        "ორი\u{00A0}სიტყვა\u{200B}უხილავი.txt",
        "ნახაზი ３.dwg",
        "Ångström ǅ ß ẞ ﬁ.txt",
    ];

    fn wide(name: &str) -> Vec<u16> {
        name.encode_utf16().collect()
    }

    /// Entries named `names`; a trailing `\` makes a folder. Entry `i` has size, created and
    /// modified `i`.
    fn entries(names: &[&str]) -> Entries {
        let mut entries = Entries::default();
        for (i, name) in names.iter().enumerate() {
            let (name, attributes) = match name.strip_suffix('\\') {
                Some(folder) => (folder, FOLDER),
                None => (*name, FILE),
            };
            let i = i as u64;
            entries.push(&wide(name), i, i, i, attributes);
        }
        entries
    }

    fn names_in(entries: &Entries, order: &[u32]) -> Vec<String> {
        order
            .iter()
            .map(|&id| entries.name(id as usize).to_string_lossy().into_owned())
            .collect()
    }

    fn sorted(entries: &Entries, sort: Sort) -> Vec<String> {
        let index = SortIndex::build(entries, &PlainTypeNames);
        names_in(entries, &index.order(entries, sort))
    }

    fn by(column: SortColumn, descending: bool) -> Sort {
        Sort {
            column,
            descending,
            folders_first: true,
        }
    }

    const COLUMNS: [SortColumn; 6] = [
        SortColumn::Name,
        SortColumn::Modified,
        SortColumn::Type,
        SortColumn::Size,
        SortColumn::Created,
        SortColumn::Extension,
    ];

    /// `StrCmpLogicalW(a, b)`: negative, 0 or positive.
    fn logical(a: &[u16], b: &[u16]) -> i32 {
        let terminated = |units: &[u16]| units.iter().copied().chain([0]).collect::<Vec<u16>>();
        let (a, b) = (terminated(a), terminated(b));
        // SAFETY: both buffers are NUL-terminated and live through the call.
        unsafe { StrCmpLogicalW(PCWSTR(a.as_ptr()), PCWSTR(b.as_ptr())) }
    }

    /// The pairs of `order` (entry ids of `entries`) that `StrCmpLogicalW` puts the other way.
    fn disagreements(entries: &Entries, order: &[u32]) -> Vec<String> {
        let mut wrong = Vec::new();
        for (i, &a) in order.iter().enumerate() {
            for &b in &order[i + 1..] {
                let (a, b) = (a as usize, b as usize);
                if logical(entries.name_utf16(a), entries.name_utf16(b)) > 0 {
                    wrong.push(format!(
                        "{:?} before {:?}",
                        entries.name(a),
                        entries.name(b)
                    ));
                }
            }
        }
        wrong
    }

    #[test]
    fn names_sort_naturally_with_folders_first() {
        let list = entries(&[
            "file10.txt",
            "File2.txt",
            "file1.txt",
            "ნახაზი 10.dwg",
            "ნახაზი 9.dwg",
            "Folder 10\\",
            "Folder 2\\",
        ]);
        assert_eq!(
            sorted(&list, Sort::default()),
            [
                "Folder 2",
                "Folder 10",
                "file1.txt",
                "File2.txt",
                "file10.txt",
                "ნახაზი 9.dwg",
                "ნახაზი 10.dwg",
            ]
        );
        assert_eq!(
            sorted(&list, by(SortColumn::Name, true)),
            [
                "Folder 10",
                "Folder 2",
                "ნახაზი 10.dwg",
                "ნახაზი 9.dwg",
                "file10.txt",
                "File2.txt",
                "file1.txt",
            ],
            "descending reverses each group, folders stay first"
        );
        let mixed = Sort {
            folders_first: false,
            ..Sort::default()
        };
        assert_eq!(
            sorted(&list, mixed),
            [
                "file1.txt",
                "File2.txt",
                "file10.txt",
                "Folder 2",
                "Folder 10",
                "ნახაზი 9.dwg",
                "ნახაზი 10.dwg",
            ]
        );
    }

    /// Spike #18: numbered Georgian names sort as Explorer shows them.
    #[test]
    fn georgian_numbers_sort_naturally() {
        let list = entries(&[
            "ნახაზი 100.dwg",
            "ნახაზი 10.dwg",
            "ნახაზი 2.dwg",
            "ნახაზი 007.dwg",
            "ნახაზი 1.dwg",
            "ნახაზი 11.dwg",
            "ნახაზი 9.dwg",
            "სართული 1 გეგმა v10.dwg",
            "სართული 12 ჭრილი.dwg",
            "სართული 1 გეგმა v2.dwg",
        ]);
        assert_eq!(
            sorted(&list, Sort::default()),
            [
                "ნახაზი 1.dwg",
                "ნახაზი 2.dwg",
                "ნახაზი 007.dwg",
                "ნახაზი 9.dwg",
                "ნახაზი 10.dwg",
                "ნახაზი 11.dwg",
                "ნახაზი 100.dwg",
                "სართული 1 გეგმა v2.dwg",
                "სართული 1 გეგმა v10.dwg",
                "სართული 12 ჭრილი.dwg",
            ]
        );
    }

    #[test]
    fn sizes_and_dates_sort_by_exact_values_and_ties_by_name() {
        let mut list = Entries::default();
        // FILETIMEs 1 tick (100 ns) apart: the same in the UI's ms, still apart here.
        let t = 134_116_992_000_000_000;
        list.push(&wide("b.txt"), 5, t + 1, t, FILE);
        list.push(&wide("a.txt"), 5, t, t + 1, FILE);
        list.push(&wide("c.txt"), 1, t + 2, t + 2, FILE);
        list.push(&wide("folder"), 0, t + 3, t - 1, FOLDER);

        assert_eq!(
            sorted(&list, by(SortColumn::Size, false)),
            ["folder", "c.txt", "a.txt", "b.txt"]
        );
        assert_eq!(
            sorted(&list, by(SortColumn::Size, true)),
            ["folder", "b.txt", "a.txt", "c.txt"]
        );
        assert_eq!(
            sorted(&list, by(SortColumn::Modified, false)),
            ["folder", "b.txt", "a.txt", "c.txt"]
        );
        assert_eq!(
            sorted(&list, by(SortColumn::Created, false)),
            ["folder", "a.txt", "b.txt", "c.txt"]
        );
        let newest_first = Sort {
            folders_first: false,
            ..by(SortColumn::Created, true)
        };
        assert_eq!(
            sorted(&list, newest_first),
            ["folder", "c.txt", "b.txt", "a.txt"]
        );
    }

    #[test]
    fn extensions_sort_naturally_and_ignore_case() {
        let list = entries(&[
            "z.TXT",
            "a.pdf",
            "noext",
            "b.txt",
            "v1.2\\",
            "m.mp4",
            "m.mp3",
            "archive.tar.gz",
            ".gitignore",
        ]);
        assert_eq!(
            sorted(&list, by(SortColumn::Extension, false)),
            [
                "v1.2",
                "noext",
                ".gitignore",
                "archive.tar.gz",
                "m.mp3",
                "m.mp4",
                "a.pdf",
                "b.txt",
                "z.TXT",
            ]
        );
        let mixed = Sort {
            folders_first: false,
            ..by(SortColumn::Extension, true)
        };
        assert_eq!(
            sorted(&list, mixed),
            [
                "z.TXT",
                "b.txt",
                "a.pdf",
                "m.mp4",
                "m.mp3",
                "archive.tar.gz",
                ".gitignore",
                "noext",
                "v1.2",
            ],
            "with folders mixed in, a folder has no extension"
        );
    }

    #[test]
    fn types_sort_by_type_name() {
        let list = entries(&["b.dwg", "a.txt", "plans\\", "noext", "c.DWG", "d.pdf"]);
        let mixed = Sort {
            folders_first: false,
            ..by(SortColumn::Type, false)
        };
        // Explorer's names for unregistered types: "DWG File" < "File" < "File folder" < …
        assert_eq!(
            sorted(&list, mixed),
            ["b.dwg", "c.DWG", "noext", "plans", "d.pdf", "a.txt"]
        );

        /// Registered names, as the Shell gives them: the column follows them, not the extension.
        struct Registered;
        impl TypeNames for Registered {
            fn type_name(&self, extension: Option<&[u16]>) -> Vec<u16> {
                let extension = extension.map(|text| String::from_utf16_lossy(text).to_lowercase());
                wide(match extension.as_deref() {
                    None => "File folder",
                    Some("dwg") => "AutoCAD Drawing",
                    Some("pdf") => "Adobe Acrobat Document",
                    Some("txt") => "Text Document",
                    Some(_) => "File",
                })
            }
        }
        let index = SortIndex::build(&list, &Registered);
        assert_eq!(
            names_in(&list, &index.order(&list, mixed)),
            ["d.pdf", "b.dwg", "c.DWG", "noext", "plans", "a.txt"]
        );
    }

    #[test]
    fn plain_type_names_are_explorers_unregistered_ones() {
        let name = |extension: Option<&str>| {
            let extension = extension.map(wide);
            String::from_utf16_lossy(&PlainTypeNames.type_name(extension.as_deref()))
        };
        assert_eq!(name(None), "File folder");
        assert_eq!(name(Some("")), "File");
        assert_eq!(name(Some("dwg")), "DWG File");
        // `toUpperCase` would turn Mkhedruli into Mtavruli; Explorer doesn't.
        assert_eq!(name(Some("ფაილი")), "ფაილი File");
    }

    #[test]
    fn the_extension_is_after_the_last_dot() {
        let text = |name: &str| String::from_utf16_lossy(extension(&wide(name)));
        assert_eq!(text("archive.tar.gz"), "gz");
        assert_eq!(text(".gitignore"), "gitignore");
        assert_eq!(text("ანგარიში.ფაილი"), "ფაილი");
        assert_eq!(text("noext"), "");
        assert_eq!(text("trailing."), "");
    }

    #[test]
    fn an_index_without_some_entries_sorts_like_one_built_without_them() {
        let list = entries(&[
            "file10.txt",
            "ნახაზი.dwg",
            "file2.txt",
            "ფოლდერი\\",
            "a.pdf",
        ]);
        let full = SortIndex::build(&list, &PlainTypeNames);
        let removed = full.without(&[1, 4]);
        let rebuilt = SortIndex::build_for(&list, vec![0, 2, 3], &PlainTypeNames);
        assert_eq!(removed.len(), 3);
        for column in COLUMNS {
            for descending in [false, true] {
                let sort = by(column, descending);
                assert_eq!(removed.order(&list, sort), rebuilt.order(&list, sort));
            }
        }
        assert_eq!(
            names_in(&list, &removed.order(&list, Sort::default())),
            ["ფოლდერი", "file2.txt", "file10.txt"]
        );
    }

    #[test]
    fn orders_are_permutations_even_with_equal_keys() {
        // "a" and "A" have the same key, so the exact name and then the id decide.
        let list = entries(&["a", "A", "a", "", "b\\"]);
        let index = SortIndex::build(&list, &PlainTypeNames);
        for column in COLUMNS {
            for descending in [false, true] {
                let order = index.order(&list, by(column, descending));
                assert_eq!(order, index.order(&list, by(column, descending)));
                let mut ids = order.clone();
                ids.sort_unstable();
                assert_eq!(ids, [0, 1, 2, 3, 4], "{column:?} {descending}");
            }
        }
        assert_eq!(sorted(&list, Sort::default()), ["b", "", "A", "a", "a"]);

        let empty = Entries::default();
        let index = SortIndex::build(&empty, &PlainTypeNames);
        assert!(index.is_empty());
        assert!(index.order(&empty, Sort::default()).is_empty());
    }

    #[test]
    fn long_names_get_keys_too() {
        // Keys of names this long don't fit the stack buffer.
        let long = "ნახაზი ".repeat(36) + "10.dwg";
        let shorter_number = "ნახაზი ".repeat(36) + "9.dwg";
        let mut key = Vec::new();
        assert!(append_sort_key(&wide(&long), &mut key));
        assert!(key.len() > 512, "{} bytes", key.len());
        let list = entries(&[&long, &shorter_number]);
        let index = SortIndex::build(&list, &PlainTypeNames);
        assert_eq!(index.order(&list, Sort::default()), [1, 0]);
    }

    /// Spike #18: the Georgian corpus against `StrCmpLogicalW`, every pair.
    #[test]
    fn georgian_corpus_agrees_with_str_cmp_logical() {
        let mut names: Vec<OsString> = GEORGIAN.iter().map(OsString::from).collect();
        let mut odd: Vec<u16> = wide("უცნაური ");
        odd.push(0xD800);
        odd.extend(wide(" სახელი.txt"));
        names.push(OsString::from_wide(&odd));
        assert_eq!(names.len() * (names.len() - 1) / 2, 780);

        let mut list = Entries::default();
        for name in &names {
            list.push(&name.encode_wide().collect::<Vec<_>>(), 0, 0, 0, FILE);
        }
        let index = SortIndex::build(&list, &PlainTypeNames);
        let wrong = disagreements(&list, &index.order(&list, Sort::default()));
        assert!(
            wrong.is_empty(),
            "{} pairs disagree: {wrong:#?}",
            wrong.len()
        );
    }

    /// The synthetic folder's names (numbers, Georgian, mixed extensions): every neighbour pair.
    #[test]
    fn synthetic_names_agree_with_str_cmp_logical() {
        let mut list = Entries::default();
        for entry in crate::synth::entries(10_000) {
            list.push(&wide(&entry.name), 0, 0, 0, FILE);
        }
        let order = SortIndex::build(&list, &PlainTypeNames).order(&list, Sort::default());
        let wrong: Vec<String> = order
            .windows(2)
            .filter_map(|pair| {
                let (a, b) = (pair[0] as usize, pair[1] as usize);
                (logical(list.name_utf16(a), list.name_utf16(b)) > 0)
                    .then(|| format!("{:?} before {:?}", list.name(a), list.name(b)))
            })
            .collect();
        assert!(
            wrong.is_empty(),
            "{} pairs disagree: {wrong:#?}",
            wrong.len()
        );
    }

    /// Names like the ones folders hold: Latin and Georgian letters (Mtavruli too), digits
    /// (full-width too), spaces and the usual punctuation.
    fn name() -> impl Strategy<Value = String> {
        let char = prop_oneof![
            4 => prop::char::range('a', 'z'),
            2 => prop::char::range('A', 'Z'),
            4 => prop::char::range('0', '9'),
            4 => prop::char::range('ა', 'ჰ'),
            1 => prop::char::range('Ა', 'Ჰ'),
            1 => prop::char::range('０', '９'),
            2 => prop::sample::select(vec![' ', '.', '-', '_', '(', ')', '[', ']', '\'', '~', '№']),
        ];
        prop::collection::vec(char, 1..12).prop_map(String::from_iter)
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(512))]

        /// Any names sort the way `StrCmpLogicalW` compares them, pair by pair.
        #[test]
        fn natural_order_agrees_with_str_cmp_logical(names in prop::collection::vec(name(), 2..24)) {
            let refs: Vec<&str> = names.iter().map(String::as_str).collect();
            let list = entries(&refs);
            let order = SortIndex::build(&list, &PlainTypeNames).order(&list, Sort::default());
            let wrong = disagreements(&list, &order);
            prop_assert!(wrong.is_empty(), "{wrong:#?}");
        }

        /// Descending is ascending reversed, with folders kept first.
        #[test]
        fn descending_reverses_each_group(
            items in prop::collection::vec((name(), any::<bool>(), 0u64..4), 0..40),
            column in prop::sample::select(COLUMNS.to_vec()),
        ) {
            let mut list = Entries::default();
            for (name, folder, value) in &items {
                let attributes = if *folder { FOLDER } else { FILE };
                list.push(&wide(name), *value, *value, *value, attributes);
            }
            let index = SortIndex::build(&list, &PlainTypeNames);
            let ascending = index.order(&list, by(column, false));
            let (folders, files): (Vec<u32>, Vec<u32>) =
                ascending.iter().partition(|&&id| list.is_dir(id as usize));
            let expected: Vec<u32> =
                folders.into_iter().rev().chain(files.into_iter().rev()).collect();
            prop_assert_eq!(index.order(&list, by(column, true)), expected);
        }
    }
}
