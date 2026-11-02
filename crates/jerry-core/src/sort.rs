//! Natural sort in Explorer's order (`StrCmpLogicalW` semantics: file2 before file10, case
//! ignored, the user's locale) with a sort key computed once per name.
//!
//! The key is Windows' own: `LCMapStringEx(LCMAP_SORTKEY | NORM_IGNORECASE | SORT_DIGITSASNUMBERS)`,
//! a byte string whose plain byte order is the locale's collation order. Keys are built in parallel
//! once per listing; after that the name order is a byte-slice sort, and every other column sorts
//! packed integers (primary value + name rank), so changing the sort never compares strings.

use std::collections::HashMap;
use std::time::Instant;

use rayon::prelude::*;
use windows::Win32::Foundation::LPARAM;
use windows::Win32::Globalization::{
    LCMAP_SORTKEY, LCMapStringEx, NORM_IGNORECASE, SORT_DIGITSASNUMBERS,
};
use windows::core::PCWSTR;

use crate::listing::Entries;

/// Flags for every sort key. `LOCALE_NAME_USER_DEFAULT` (a null name) follows the user's locale,
/// as Explorer does.
const KEY_FLAGS: u32 = LCMAP_SORTKEY | NORM_IGNORECASE.0 | SORT_DIGITSASNUMBERS.0;

/// Names per parallel work item when building keys.
const KEY_CHUNK: usize = 2048;

/// A column the list can be sorted by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortColumn {
    Name,
    Size,
    Modified,
    /// By extension. Explorer sorts by the type's description, which needs the Shell; the spike
    /// uses the extension.
    Type,
}

/// The natural-sort key of `name` (UTF-16), appended to `out`. Returns false if Windows refused.
pub fn append_sort_key(name: &[u16], out: &mut Vec<u8>) -> bool {
    if name.is_empty() {
        return true;
    }
    let mut buffer = [0u16; 1024];
    // SAFETY: `name` and `buffer` are valid slices; for LCMAP_SORTKEY the destination length is
    // read as bytes, and the buffer holds twice as many bytes as reported.
    let mut written = unsafe {
        LCMapStringEx(
            PCWSTR::null(),
            KEY_FLAGS,
            name,
            Some(&mut buffer[..]),
            None,
            None,
            LPARAM(0),
        )
    };
    if written <= 0 {
        // Too long for the stack buffer: ask for the size and retry on the heap.
        // SAFETY: as above, with no destination (returns the required byte count).
        let needed =
            unsafe { LCMapStringEx(PCWSTR::null(), KEY_FLAGS, name, None, None, None, LPARAM(0)) };
        if needed <= 0 {
            return false;
        }
        let mut heap = vec![0u16; needed as usize];
        // SAFETY: as above.
        written = unsafe {
            LCMapStringEx(
                PCWSTR::null(),
                KEY_FLAGS,
                name,
                Some(&mut heap[..]),
                None,
                None,
                LPARAM(0),
            )
        };
        if written <= 0 {
            return false;
        }
        out.extend_from_slice(units_as_bytes(&heap, written as usize));
        return true;
    }
    out.extend_from_slice(units_as_bytes(&buffer, written as usize));
    true
}

fn units_as_bytes(units: &[u16], len: usize) -> &[u8] {
    let len = len.min(units.len() * 2);
    // SAFETY: u16 has no padding and u8 has alignment 1, so the first `len` bytes of `units` are a
    // valid byte slice (`len` is clamped to the slice's byte length).
    unsafe { std::slice::from_raw_parts(units.as_ptr().cast::<u8>(), len) }
}

/// Sort keys for every name of a listing, in one arena.
struct SortKeys {
    bytes: Vec<u8>,
    ends: Vec<u32>,
}

impl SortKeys {
    fn build(entries: &Entries) -> Self {
        let len = entries.len();
        let parts: Vec<(Vec<u8>, Vec<u32>)> = (0..len.div_ceil(KEY_CHUNK))
            .into_par_iter()
            .map(|chunk| {
                let ids = chunk * KEY_CHUNK..((chunk + 1) * KEY_CHUNK).min(len);
                let mut bytes = Vec::with_capacity(ids.len() * 64);
                let mut ends = Vec::with_capacity(ids.len());
                for id in ids {
                    let name = entries.name_utf16(id);
                    if !append_sort_key(name, &mut bytes) {
                        // Fall back to the raw name: still a stable, deterministic order.
                        bytes.extend(name.iter().flat_map(|unit| unit.to_be_bytes()));
                    }
                    ends.push(bytes.len() as u32);
                }
                (bytes, ends)
            })
            .collect();

        let total = parts.iter().map(|(bytes, _)| bytes.len()).sum();
        let mut keys = SortKeys {
            bytes: Vec::with_capacity(total),
            ends: Vec::with_capacity(len),
        };
        for (bytes, ends) in parts {
            let base = keys.bytes.len() as u32;
            keys.bytes.extend_from_slice(&bytes);
            keys.ends.extend(ends.into_iter().map(|end| base + end));
        }
        keys
    }

    fn get(&self, id: usize) -> &[u8] {
        let start = if id == 0 { 0 } else { self.ends[id - 1] };
        &self.bytes[start as usize..self.ends[id] as usize]
    }
}

/// How long [`SortIndex::build`] took, by phase.
#[derive(Debug, Clone, Copy, Default)]
pub struct IndexTimings {
    pub keys_ms: f64,
    pub name_sort_ms: f64,
    pub ranks_ms: f64,
}

/// Everything needed to sort a listing by any column without comparing strings again.
#[derive(Debug, Default)]
pub struct SortIndex {
    /// Entry ids in ascending natural name order.
    by_name: Vec<u32>,
    /// `name_rank[id]`: the entry's position in `by_name`.
    name_rank: Vec<u32>,
    /// `type_rank[id]`: rank of the entry's extension in natural order (folders 0, no extension 1).
    type_rank: Vec<u32>,
    pub timings: IndexTimings,
}

impl SortIndex {
    /// Builds keys and ranks for `entries` (parallel, on the rayon pool).
    pub fn build(entries: &Entries) -> Self {
        let started = Instant::now();
        let keys = SortKeys::build(entries);
        let keys_ms = ms_since(started);

        let started = Instant::now();
        let mut by_name: Vec<u32> = (0..entries.len() as u32).collect();
        by_name.par_sort_unstable_by(|&a, &b| {
            let (a, b) = (a as usize, b as usize);
            keys.get(a)
                .cmp(keys.get(b))
                .then_with(|| entries.name_utf16(a).cmp(entries.name_utf16(b)))
                .then(a.cmp(&b))
        });
        let name_sort_ms = ms_since(started);

        let started = Instant::now();
        let mut name_rank = vec![0u32; entries.len()];
        for (rank, &id) in by_name.iter().enumerate() {
            name_rank[id as usize] = rank as u32;
        }
        let type_rank = type_ranks(entries);
        let ranks_ms = ms_since(started);

        SortIndex {
            by_name,
            name_rank,
            type_rank,
            timings: IndexTimings {
                keys_ms,
                name_sort_ms,
                ranks_ms,
            },
        }
    }

    /// Entry ids in display order. Ties break by name; folders come first if `folders_first`
    /// (in both directions, as in Explorer).
    pub fn order(
        &self,
        entries: &Entries,
        column: SortColumn,
        descending: bool,
        folders_first: bool,
    ) -> Vec<u32> {
        // One u128 per entry: group (bit 96) | primary value (bits 32..96) | name rank (bits 0..32).
        // The rank is unique, so sorting plain integers gives a total order and the low 32 bits
        // map back to the entry id.
        let mut packed: Vec<u128> = (0..entries.len())
            .into_par_iter()
            .with_min_len(4096)
            .map(|id| {
                let group = u128::from(!(folders_first && entries.is_dir(id)));
                let primary = match column {
                    SortColumn::Name => 0,
                    SortColumn::Size => entries.sizes[id],
                    SortColumn::Modified => entries.modified[id],
                    SortColumn::Type => u64::from(self.type_rank[id]),
                };
                let rank = self.name_rank[id];
                let (primary, rank) = if descending {
                    (!primary, !rank)
                } else {
                    (primary, rank)
                };
                (group << 96) | (u128::from(primary) << 32) | u128::from(rank)
            })
            .collect();
        packed.par_sort_unstable();
        let reversed = descending;
        packed
            .into_par_iter()
            .with_min_len(4096)
            .map(|key| {
                let rank = key as u32;
                let rank = if reversed { !rank } else { rank };
                self.by_name[rank as usize]
            })
            .collect()
    }
}

/// Rank of each entry's extension in natural order: folders 0, files without one 1.
fn type_ranks(entries: &Entries) -> Vec<u32> {
    const DOT: u16 = b'.' as u16;
    let extension = |id: usize| -> &[u16] {
        let name = entries.name_utf16(id);
        match name.iter().rposition(|&unit| unit == DOT) {
            Some(dot) => &name[dot + 1..],
            None => &[],
        }
    };

    // Unique extensions as written (no allocation per entry); a folder holds only a few dozen.
    let mut index_of: HashMap<&[u16], u32> = HashMap::new();
    let mut extensions: Vec<&[u16]> = Vec::new();
    let mut extension_of = vec![u32::MAX; entries.len()];
    for (id, slot) in extension_of.iter_mut().enumerate() {
        if entries.is_dir(id) {
            continue;
        }
        let text = extension(id);
        let next = extensions.len() as u32;
        *slot = *index_of.entry(text).or_insert_with(|| {
            extensions.push(text);
            next
        });
    }

    // Rank them by natural sort key. The key ignores case, so "JPG" and "jpg" share a rank.
    let mut keyed: Vec<(Vec<u8>, u32)> = extensions
        .iter()
        .enumerate()
        .map(|(index, text)| {
            let mut key = Vec::new();
            append_sort_key(text, &mut key);
            (key, index as u32)
        })
        .collect();
    keyed.sort_unstable();
    let mut rank_of_extension = vec![0u32; extensions.len()];
    let mut rank = 0;
    for (position, (key, index)) in keyed.iter().enumerate() {
        if position == 0 || keyed[position - 1].0 != *key {
            rank += 1;
        }
        // An empty extension's key is empty, so it ranks first: 1, right after folders.
        rank_of_extension[*index as usize] = rank;
    }

    extension_of
        .into_iter()
        .map(|index| {
            if index == u32::MAX {
                0
            } else {
                rank_of_extension[index as usize]
            }
        })
        .collect()
}

fn ms_since(started: Instant) -> f64 {
    started.elapsed().as_secs_f64() * 1000.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::listing::ATTRIBUTE_DIRECTORY;

    fn entries(names: &[(&str, bool, u64)]) -> Entries {
        let mut entries = Entries::default();
        for (index, (name, is_dir, size)) in names.iter().enumerate() {
            let units: Vec<u16> = name.encode_utf16().collect();
            let attributes = if *is_dir { ATTRIBUTE_DIRECTORY } else { 0 };
            entries.push(&units, *size, index as u64, attributes);
        }
        entries
    }

    fn names_in(entries: &Entries, order: &[u32]) -> Vec<String> {
        order
            .iter()
            .map(|&id| entries.name(id as usize).into_string().unwrap())
            .collect()
    }

    #[test]
    fn natural_order_numbers_case_and_georgian() {
        let list = entries(&[
            ("file10.txt", false, 0),
            ("File2.txt", false, 0),
            ("file1.txt", false, 0),
            ("ნახაზი 10.dwg", false, 0),
            ("ნახაზი 9.dwg", false, 0),
            ("Folder 10", true, 0),
            ("Folder 2", true, 0),
        ]);
        let index = SortIndex::build(&list);
        let order = index.order(&list, SortColumn::Name, false, true);
        assert_eq!(
            names_in(&list, &order),
            [
                "Folder 2",
                "Folder 10",
                "file1.txt",
                "File2.txt",
                "file10.txt",
                "ნახაზი 9.dwg",
                "ნახაზი 10.dwg"
            ]
        );
        let order = index.order(&list, SortColumn::Name, true, true);
        assert_eq!(
            names_in(&list, &order),
            [
                "Folder 10",
                "Folder 2",
                "ნახაზი 10.dwg",
                "ნახაზი 9.dwg",
                "file10.txt",
                "File2.txt",
                "file1.txt"
            ]
        );
    }

    #[test]
    fn size_ties_break_by_name() {
        let list = entries(&[
            ("b", false, 5),
            ("a", false, 5),
            ("c", false, 1),
            ("dir", true, 0),
        ]);
        let index = SortIndex::build(&list);
        let order = index.order(&list, SortColumn::Size, false, true);
        assert_eq!(names_in(&list, &order), ["dir", "c", "a", "b"]);
        let order = index.order(&list, SortColumn::Size, true, false);
        assert_eq!(names_in(&list, &order), ["b", "a", "c", "dir"]);
    }

    #[test]
    fn type_sorts_by_extension() {
        let list = entries(&[
            ("z.TXT", false, 0),
            ("a.pdf", false, 0),
            ("noext", false, 0),
            ("b.txt", false, 0),
        ]);
        let index = SortIndex::build(&list);
        let order = index.order(&list, SortColumn::Type, false, true);
        assert_eq!(
            names_in(&list, &order),
            ["noext", "a.pdf", "b.txt", "z.TXT"]
        );
    }

    /// Spike #18: numbered Georgian names sort as Explorer shows them.
    #[test]
    fn georgian_numbers_sort_naturally() {
        let names = [
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
        ];
        let list = entries(&names.map(|name| (name, false, 0)));
        let index = SortIndex::build(&list);
        let order = index.order(&list, SortColumn::Name, false, true);
        assert_eq!(
            names_in(&list, &order),
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

    /// Spike #18: the Georgian corpus (mixed scripts, Mtavruli, historic letters, emoji, full-width
    /// digits) against `StrCmpLogicalW`, in both directions of every pair.
    #[test]
    fn georgian_corpus_agrees_with_str_cmp_logical() {
        use std::os::windows::ffi::OsStrExt;
        use windows::Win32::UI::Shell::StrCmpLogicalW;
        use windows::core::PCWSTR;

        let names = crate::synth::georgian::names();
        let mut list = Entries::default();
        for name in &names {
            let units: Vec<u16> = name.encode_wide().collect();
            list.push(&units, 0, 0, 0);
        }
        let index = SortIndex::build(&list);
        let order = index.order(&list, SortColumn::Name, false, false);
        let wide = |id: u32| -> Vec<u16> {
            let mut units = list.name_utf16(id as usize).to_vec();
            units.push(0);
            units
        };
        let mut disagreements = Vec::new();
        for (i, &a) in order.iter().enumerate() {
            for &b in &order[i + 1..] {
                let (wa, wb) = (wide(a), wide(b));
                // SAFETY: both buffers are NUL-terminated and live through the call.
                let cmp = unsafe { StrCmpLogicalW(PCWSTR(wa.as_ptr()), PCWSTR(wb.as_ptr())) };
                if cmp > 0 {
                    disagreements.push(format!(
                        "{:?} before {:?}",
                        list.name(a as usize),
                        list.name(b as usize)
                    ));
                }
            }
        }
        assert!(
            disagreements.is_empty(),
            "{} pairs disagree: {disagreements:#?}",
            disagreements.len()
        );
    }

    /// Our key order against `StrCmpLogicalW` on the synthetic names: every adjacent pair must be
    /// in order. Prints disagreements so the spike can report them.
    #[test]
    fn agrees_with_str_cmp_logical() {
        use windows::Win32::UI::Shell::StrCmpLogicalW;
        use windows::core::HSTRING;

        let synth = crate::synth::entries(10_000);
        let mut list = Entries::default();
        for entry in &synth {
            let units: Vec<u16> = entry.name.encode_utf16().collect();
            list.push(&units, 0, 0, 0);
        }
        let index = SortIndex::build(&list);
        let names = names_in(&list, &index.order(&list, SortColumn::Name, false, false));
        let mut disagreements = Vec::new();
        for pair in names.windows(2) {
            // SAFETY: both HSTRINGs are NUL-terminated and live through the call.
            let cmp = unsafe { StrCmpLogicalW(&HSTRING::from(&pair[0]), &HSTRING::from(&pair[1])) };
            if cmp > 0 {
                disagreements.push(format!("{} > {}", pair[0], pair[1]));
            }
        }
        assert!(
            disagreements.is_empty(),
            "{} of {} pairs disagree, e.g. {:?}",
            disagreements.len(),
            names.len() - 1,
            &disagreements[..disagreements.len().min(10)]
        );
    }
}
