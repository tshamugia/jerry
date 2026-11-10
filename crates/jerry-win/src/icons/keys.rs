//! Icon keys: what a listing entry's icon depends on, as one `u32` per entry that the listing
//! batch carries. Deriving one is pure (name and attributes, no Shell call and no disk access),
//! so it costs the listing almost nothing.
//!
//! - [`OWN`]: the entry has its own icon (.exe, .lnk, .ico, .url, Tech PRD «აიკონები»): the UI
//!   asks for it by entry.
//! - [`FOLDER`], [`FILE`] (a file with no extension), and from [`FIRST_TYPE`] on, one key per
//!   extension, numbered in the order this process first saw them. Every file of a type shares
//!   its icon, so the UI and WebView2 cache it once.
//!
//! A cloud-only file (OneDrive "online only") never gets [`OWN`]: its own icon would mean reading
//! its content, which downloads it. It shows its type's icon instead.

use std::collections::HashMap;
use std::sync::{LazyLock, PoisonError, RwLock};

use jerry_core::listing::Entries;

/// The entry's own icon: ask for it by entry.
pub const OWN: u32 = 0;
/// A folder.
pub const FOLDER: u32 = 1;
/// A file with no extension.
pub const FILE: u32 = 2;
/// The first extension's key.
pub const FIRST_TYPE: u32 = 3;

/// Most extensions one process numbers; past that, new ones get [`FILE`]'s icon.
const MAX_TYPES: usize = 65_536;

const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
const FILE_ATTRIBUTE_OFFLINE: u32 = 0x1000;
const FILE_ATTRIBUTE_RECALL_ON_OPEN: u32 = 0x4_0000;
const FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS: u32 = 0x40_0000;

/// Attributes of a file whose content isn't on this device (OneDrive's online-only files and
/// other cloud placeholders): reading it would download it.
pub const CLOUD_ONLY: u32 =
    FILE_ATTRIBUTE_OFFLINE | FILE_ATTRIBUTE_RECALL_ON_OPEN | FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS;

/// Extensions whose files each have their own icon, lowercase.
const OWN_ICON: [&str; 4] = ["exe", "lnk", "ico", "url"];

/// What a key other than [`OWN`] names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IconType {
    Folder,
    /// A file with no extension.
    File,
    /// A file with this extension: lowercase (ASCII letters), without the dot.
    Extension(Box<[u16]>),
}

/// The extensions numbered so far: key `FIRST_TYPE + i` is `names[i]`.
#[derive(Default)]
struct Types {
    keys: HashMap<Box<[u16]>, u32>,
    names: Vec<Box<[u16]>>,
}

static TYPES: LazyLock<RwLock<Types>> = LazyLock::new(RwLock::default);

/// The icon key of an entry named `name` (UTF-16, as listed) with `attributes`.
pub fn key(name: &[u16], attributes: u32) -> u32 {
    let mut lower = Vec::new();
    match classify(name, attributes, &mut lower) {
        Class::Key(key) => key,
        Class::Extension => intern(&lower),
    }
}

/// The icon keys of `entries`, in order: what the listing batch sends.
pub fn keys(entries: &Entries) -> Vec<u32> {
    const UNKNOWN: u32 = u32::MAX;
    let mut lower = Vec::new();
    let mut keys = Vec::with_capacity(entries.len());
    let mut unknown = false;
    {
        let types = TYPES.read().unwrap_or_else(PoisonError::into_inner);
        for (id, &attributes) in entries.attributes().iter().enumerate() {
            let key = match classify(entries.name_utf16(id), attributes, &mut lower) {
                Class::Key(key) => key,
                Class::Extension => {
                    types
                        .keys
                        .get(lower.as_slice())
                        .copied()
                        .unwrap_or_else(|| {
                            unknown = true;
                            UNKNOWN
                        })
                }
            };
            keys.push(key);
        }
    }
    // New extensions: one write lock for the batch, and only when there are any.
    if unknown {
        let mut types = TYPES.write().unwrap_or_else(PoisonError::into_inner);
        for (id, key) in keys.iter_mut().enumerate() {
            if *key == UNKNOWN {
                let attributes = entries.attributes()[id];
                if let Class::Extension = classify(entries.name_utf16(id), attributes, &mut lower) {
                    *key = types.intern(&lower);
                }
            }
        }
    }
    keys
}

/// The key of the type icon of an item named `name` with `attributes`: never [`OWN`], for where
/// there is no listing entry to ask by (the conflict window, #51).
pub fn type_key(name: &[u16], attributes: u32) -> u32 {
    match icon_type_of(name, attributes) {
        IconType::Folder => FOLDER,
        IconType::File => FILE,
        IconType::Extension(lower) => intern(&lower),
    }
}

/// What `key` names, or `None` for [`OWN`] and keys this process never gave out.
pub fn icon_type(key: u32) -> Option<IconType> {
    match key {
        OWN => None,
        FOLDER => Some(IconType::Folder),
        FILE => Some(IconType::File),
        _ => {
            let types = TYPES.read().unwrap_or_else(PoisonError::into_inner);
            let name = types.names.get((key - FIRST_TYPE) as usize)?;
            Some(IconType::Extension(name.clone()))
        }
    }
}

/// True if a file named `name` with `attributes` shows its own icon rather than its type's.
pub fn has_own_icon(name: &[u16], attributes: u32) -> bool {
    matches!(classify(name, attributes, &mut Vec::new()), Class::Key(OWN))
}

/// The extension of `name` as Windows sees it (`PathFindExtensionW`): after the last dot, unless
/// a space follows that dot. `None` if there is none. ".gitignore" is all extension.
pub fn extension(name: &[u16]) -> Option<&[u16]> {
    const DOT: u16 = b'.' as u16;
    const SPACE: u16 = b' ' as u16;
    let dot = name.iter().rposition(|&unit| unit == DOT)?;
    let extension = &name[dot + 1..];
    (!extension.is_empty() && !extension.contains(&SPACE)).then_some(extension)
}

/// The type icon of an entry, whether or not it has its own: what it shows when its own can't
/// be read.
pub fn icon_type_of(name: &[u16], attributes: u32) -> IconType {
    if attributes & FILE_ATTRIBUTE_DIRECTORY != 0 {
        return IconType::Folder;
    }
    match extension(name) {
        Some(extension) => {
            IconType::Extension(extension.iter().map(|&unit| ascii_lower(unit)).collect())
        }
        None => IconType::File,
    }
}

/// Lowercases ASCII letters only. Extensions differ in case mostly in ASCII; a non-ASCII one in
/// another case just gets a key of its own, with the same icon.
pub(super) fn ascii_lower(unit: u16) -> u16 {
    match u8::try_from(unit) {
        Ok(byte) => u16::from(byte.to_ascii_lowercase()),
        Err(_) => unit,
    }
}

enum Class {
    Key(u32),
    /// A file with an extension; the lowercase extension is in the caller's buffer.
    Extension,
}

fn classify(name: &[u16], attributes: u32, lower: &mut Vec<u16>) -> Class {
    if attributes & FILE_ATTRIBUTE_DIRECTORY != 0 {
        return Class::Key(FOLDER);
    }
    let Some(extension) = extension(name) else {
        return Class::Key(FILE);
    };
    lower.clear();
    lower.extend(extension.iter().map(|&unit| ascii_lower(unit)));
    let own = OWN_ICON
        .iter()
        .any(|own| own.encode_utf16().eq(lower.iter().copied()));
    if own && attributes & CLOUD_ONLY == 0 {
        Class::Key(OWN)
    } else {
        Class::Extension
    }
}

fn intern(lower: &[u16]) -> u32 {
    if let Some(&key) = TYPES
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .keys
        .get(lower)
    {
        return key;
    }
    TYPES
        .write()
        .unwrap_or_else(PoisonError::into_inner)
        .intern(lower)
}

impl Types {
    fn intern(&mut self, lower: &[u16]) -> u32 {
        if let Some(&key) = self.keys.get(lower) {
            return key;
        }
        if self.names.len() >= MAX_TYPES {
            return FILE;
        }
        let key = FIRST_TYPE + self.names.len() as u32;
        self.names.push(lower.into());
        self.keys.insert(lower.into(), key);
        key
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().collect()
    }

    fn key_of(name: &str, attributes: u32) -> u32 {
        key(&wide(name), attributes)
    }

    const NORMAL: u32 = 0x20;

    #[test]
    fn extensions_follow_windows() {
        let ext = |name: &str| extension(&wide(name)).map(String::from_utf16_lossy);
        assert_eq!(ext("ნახაზი 1.dwg").as_deref(), Some("dwg"));
        assert_eq!(ext("Project_v1.2.34.zip").as_deref(), Some("zip"));
        assert_eq!(ext(".gitignore").as_deref(), Some("gitignore"));
        assert_eq!(ext("README"), None);
        assert_eq!(ext("file."), None);
        assert_eq!(ext("Report v1. final"), None, "a space after the dot");
    }

    #[test]
    fn files_of_a_type_share_a_key_whatever_the_case() {
        let dwg = key_of("ნახაზი 1.dwg", NORMAL);
        assert!(dwg >= FIRST_TYPE);
        assert_eq!(key_of("Plan 7A.DWG", NORMAL), dwg);
        assert_ne!(key_of("ხელშეკრულება.pdf", NORMAL), dwg);
        assert_eq!(
            icon_type(dwg),
            Some(IconType::Extension(wide("dwg").into()))
        );
    }

    #[test]
    fn folders_and_bare_files_have_fixed_keys() {
        assert_eq!(key_of("პროექტი.v2", 0x10), FOLDER);
        assert_eq!(key_of("README", NORMAL), FILE);
        assert_eq!(icon_type(FOLDER), Some(IconType::Folder));
        assert_eq!(icon_type(FILE), Some(IconType::File));
        assert_eq!(icon_type(OWN), None);
        assert_eq!(icon_type(u32::MAX), None, "never given out");
    }

    #[test]
    fn programs_shortcuts_and_icons_have_their_own_unless_cloud_only() {
        for name in ["setup.exe", "AutoCAD.lnk", "logo.ICO", "Insta.url"] {
            assert_eq!(key_of(name, NORMAL), OWN, "{name}");
            assert!(has_own_icon(&wide(name), NORMAL));
        }
        let online_only = NORMAL | FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS;
        let exe = key_of("setup.exe", online_only);
        assert!(exe >= FIRST_TYPE, "a type key, so nothing is downloaded");
        assert_eq!(
            icon_type(exe),
            Some(IconType::Extension(wide("exe").into()))
        );
        assert!(!has_own_icon(&wide("setup.exe"), online_only));
        assert_ne!(key_of("setup.exe", NORMAL | FILE_ATTRIBUTE_OFFLINE), OWN);
    }

    #[test]
    fn batch_keys_match_single_keys() {
        let mut entries = Entries::default();
        let names = ["a.txt", "b.TXT", "c.exe", "დ", "e.json-batch-test", "F"];
        let attributes = [NORMAL, NORMAL, NORMAL, NORMAL, NORMAL, 0x10];
        for (name, attributes) in names.iter().zip(attributes) {
            entries.push(&wide(name), 0, 0, 0, attributes);
        }
        let batch = keys(&entries);
        let single: Vec<u32> = names
            .iter()
            .zip(attributes)
            .map(|(name, attributes)| key_of(name, attributes))
            .collect();
        assert_eq!(batch, single);
        assert_eq!(batch[0], batch[1]);
        assert_eq!(batch[2..], [OWN, FILE, batch[4], FOLDER]);
    }
}
