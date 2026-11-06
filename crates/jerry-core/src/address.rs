//! What the address bar's text names (#33): a folder, or a file in a folder, from a path typed or
//! pasted in any form Explorer takes. The text may be quoted (Explorer's "Copy as path"), a
//! `file:` URL, hold `%VARIABLES%`, start with a known folder's short name (`Downloads\Plans`),
//! be relative to the folder shown (`Drawings`, `..\Other`), be a drive alone (`D:`) or a share
//! (`\\server\share`). And the subfolders that complete a partly typed path.
//!
//! A relative name is tried against the folder shown first, then as a known folder, as Explorer
//! does: a subfolder named `Downloads` wins over the user's Downloads.

use std::ffi::OsString;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::fs::MetadataExt;
use std::path::{Path, PathBuf};

use crate::error::{ErrorCode, JerryError};
use crate::path::long_path;
use crate::sort::append_sort_key;

/// `FILE_ATTRIBUTE_*` flags read here.
const DIRECTORY: u32 = 0x10;
const HIDDEN_OR_SYSTEM: u32 = 0x2 | 0x4;

/// What the text names, as exact long paths.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    /// The folder to open.
    pub folder: PathBuf,
    /// A file in it to select: the text named a file.
    pub select: Option<OsString>,
}

/// Where a relative or short name may lead, for [`resolve`] and [`complete`].
pub struct Context<'a> {
    /// The folder shown, for relative paths.
    pub base: Option<&'a Path>,
    /// A known folder by its short name (`Downloads`), case-insensitive.
    pub known: &'a dyn Fn(&str) -> Option<PathBuf>,
    /// An environment variable's value.
    pub var: &'a dyn Fn(&str) -> Option<OsString>,
}

/// The text as a path is written: trimmed, without the quotes Explorer's "Copy as path" adds, and
/// a `file:` URL as its path (`file:///C:/a%20b` → `C:\a b`, `file://server/share` →
/// `\\server\share`).
pub fn clean(text: &str) -> String {
    let text = text.trim();
    let text = text
        .strip_prefix('"')
        .and_then(|inner| inner.strip_suffix('"'))
        .unwrap_or(text)
        .trim();
    let Some(url) = text
        .get(..5)
        .filter(|scheme| scheme.eq_ignore_ascii_case("file:"))
        .map(|_| &text[5..])
    else {
        return text.to_owned();
    };
    let path = percent_decoded(url);
    let path = if let Some(local) = path.strip_prefix("///") {
        local.to_owned()
    } else if let Some(share) = path.strip_prefix("//") {
        format!(r"\\{share}")
    } else {
        path
    };
    path.replace('/', r"\")
}

fn percent_decoded(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = bytes.get(i + 1..i + 3).and_then(|pair| {
            std::str::from_utf8(pair)
                .ok()
                .and_then(|pair| u8::from_str_radix(pair, 16).ok())
        });
        match (bytes[i], hex) {
            (b'%', Some(byte)) => {
                out.push(byte);
                i += 3;
            }
            (byte, _) => {
                out.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// `%NAME%` replaced by the variable's value; an unknown name stays as typed, as in Explorer.
pub fn expand(text: &str, var: &dyn Fn(&str) -> Option<OsString>) -> OsString {
    let mut out = OsString::new();
    let mut rest = text;
    while let Some(start) = rest.find('%') {
        let after = &rest[start + 1..];
        let Some(end) = after.find('%') else { break };
        out.push(&rest[..start]);
        let name = &after[..end];
        match var(name).filter(|_| !name.is_empty()) {
            Some(value) => out.push(value),
            None => out.push(format!("%{name}%")),
        }
        rest = &after[end + 1..];
    }
    out.push(rest);
    out
}

/// The paths `text` (already [`clean`]) may name, most likely first, before looking at the disk:
/// an absolute path; or, for a relative one, the folder shown joined with it, then the known
/// folder its first part names joined with the rest. `D:` alone is the drive's root.
pub fn candidates(text: &str, context: &Context<'_>) -> Vec<PathBuf> {
    let expanded = PathBuf::from(expand(text, context.var));
    if is_drive(text) {
        return vec![PathBuf::from(format!(r"{text}\"))];
    }
    if expanded.is_absolute() {
        return vec![expanded];
    }
    let mut paths = Vec::new();
    if let Some(base) = context.base {
        paths.push(base.join(&expanded));
    }
    let (first, rest) = match text.find(['\\', '/']) {
        Some(at) => (&text[..at], text[at + 1..].trim_start_matches(['\\', '/'])),
        None => (text, ""),
    };
    if let Some(known) = (context.known)(first) {
        paths.push(if rest.is_empty() {
            known
        } else {
            known.join(expand(rest, context.var))
        });
    }
    paths
}

/// `C:` alone.
fn is_drive(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.len() == 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

/// What `text` names: a folder, or a file in its folder. Fails with `InvalidName` for empty text,
/// and with the error of the likeliest path (`NotFound`, `AccessDenied`, `DeviceGone`) when none
/// of them is there. Blocking IO: a share that doesn't answer takes Windows' own timeout.
pub fn resolve(text: &str, context: &Context<'_>) -> Result<Resolved, JerryError> {
    let text = clean(text);
    if text.is_empty() {
        return Err(JerryError::new(ErrorCode::InvalidName));
    }
    let mut first_error = None;
    for path in candidates(&text, context) {
        let path = long_path(&path)?;
        match std::fs::metadata(&path) {
            Ok(meta) if meta.is_dir() => {
                return Ok(Resolved {
                    folder: path,
                    select: None,
                });
            }
            Ok(_) => {
                let (Some(folder), Some(name)) = (path.parent(), path.file_name()) else {
                    continue;
                };
                return Ok(Resolved {
                    folder: folder.to_path_buf(),
                    select: Some(name.to_os_string()),
                });
            }
            Err(error) => {
                first_error.get_or_insert_with(|| JerryError::from(error).at(&path));
            }
        }
    }
    Err(first_error.unwrap_or_else(|| JerryError::new(ErrorCode::NotFound).at(text)))
}

/// The subfolders of `folder`, by name in natural order (`StrCmpLogicalW`, as the file list
/// sorts), hidden and system ones only with `show_hidden`. Blocking IO.
pub fn subfolders(folder: &Path, show_hidden: bool) -> Result<Vec<OsString>, JerryError> {
    let folder = long_path(folder)?;
    let entries =
        std::fs::read_dir(&folder).map_err(|error| JerryError::from(error).at(&folder))?;
    let mut keyed: Vec<(Vec<u8>, OsString)> = Vec::new();
    for entry in entries.flatten() {
        // From the folder's own listing: no extra IO per entry.
        let Ok(meta) = entry.metadata() else { continue };
        let attributes = meta.file_attributes();
        if attributes & DIRECTORY == 0 || (!show_hidden && attributes & HIDDEN_OR_SYSTEM != 0) {
            continue;
        }
        let name = entry.file_name();
        let units: Vec<u16> = name.encode_wide().collect();
        let mut key = Vec::new();
        if !append_sort_key(&units, &mut key) {
            key = units.iter().flat_map(|unit| unit.to_be_bytes()).collect();
        }
        keyed.push((key, name));
    }
    keyed.sort();
    Ok(keyed.into_iter().map(|(_, name)| name).collect())
}

/// Completions for a partly typed path, at most `limit`: the subfolders whose names start with
/// what follows its last separator, case-insensitive, written as the text writes the rest (so
/// `%APPDATA%\Mi` completes to `%APPDATA%\Microsoft`). With no separator yet, the known folders'
/// short names in `known_names` and the folder shown's subfolders. Nothing when the folder can't
/// be read. Blocking IO.
pub fn complete(
    text: &str,
    context: &Context<'_>,
    known_names: &[&str],
    show_hidden: bool,
    limit: usize,
) -> Vec<String> {
    let text = clean(text);
    let starts =
        |name: &str, partial: &str| name.to_lowercase().starts_with(&partial.to_lowercase());
    let mut out: Vec<String> = Vec::new();
    match text.rfind(['\\', '/']) {
        None if is_drive(&text) || text.is_empty() => {}
        None => {
            if let Some(base) = context.base {
                for name in subfolders(base, show_hidden).unwrap_or_default() {
                    let name = name.to_string_lossy().into_owned();
                    if starts(&name, &text) {
                        out.push(name);
                    }
                }
            }
            for name in known_names {
                let taken = out.iter().any(|each| each.eq_ignore_ascii_case(name));
                if starts(name, &text) && !taken {
                    out.push((*name).to_owned());
                }
            }
        }
        Some(at) => {
            let (typed, partial) = text.split_at(at + 1);
            let folder = candidates(typed.trim_end_matches(['\\', '/']), context)
                .into_iter()
                .find(|folder| long_path(folder).is_ok_and(|path| path.is_dir()));
            if let Some(folder) = folder {
                for name in subfolders(&folder, show_hidden).unwrap_or_default() {
                    let name = name.to_string_lossy();
                    if starts(&name, partial) {
                        out.push(format!("{typed}{name}"));
                    }
                }
            }
        }
    }
    out.truncate(limit);
    out
}
