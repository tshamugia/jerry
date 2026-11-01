//! Spikes #13 and #14 only: a fixed sample folder to drag files out of and drop files into.
//! Real listing is #24, real transfers are the transfer engine's.
//!
//! The folder lives under the user's temp dir. Seed files are created when missing and never
//! overwritten, and anything the user adds (a real DWG, say) is listed too. Drops only ever
//! **copy** into it: nothing here deletes or moves a file the user dragged in.

use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::fs::OpenOptionsExt;
use std::path::{Component, Path, PathBuf};

/// One item in the sample folder.
#[derive(Clone, Debug)]
pub struct Entry {
    pub path: PathBuf,
    pub name: OsString,
    pub is_dir: bool,
    pub size: u64,
}

/// A minimal valid one-page PDF, so PDF targets (Outlook preview, Chrome, Telegram) open it.
const PDF: &[u8] = b"%PDF-1.4
1 0 obj<</Type/Catalog/Pages 2 0 R>>endobj
2 0 obj<</Type/Pages/Kids[3 0 R]/Count 1>>endobj
3 0 obj<</Type/Page/Parent 2 0 R/MediaBox[0 0 300 120]/Contents 4 0 R/Resources<</Font<</F1 5 0 R>>>>>>endobj
4 0 obj<</Length 42>>stream
BT /F1 18 Tf 30 60 Td (Jerry spike) Tj ET
endstream endobj
5 0 obj<</Type/Font/Subtype/Type1/BaseFont/Helvetica>>endobj
trailer<</Root 1 0 R>>
%%EOF
";

/// Seed files: Georgian and plain names, a DWG-like and a PDF-like file, a few folders to
/// drop into (tabs, breadcrumb and sidebar point at them).
fn seeds() -> [(&'static str, &'static [u8]); 7] {
    [
        // Only the DWG signature: AutoCAD sees a DWG and reports it as damaged, which still
        // proves the file arrived. Copy a real drawing into the folder to test opening.
        ("Drawings\\ნახაზი — სართული 1.dwg", b"AC1032\0\0\0\0\0\0"),
        (
            "Drawings\\2026\\შენიშვნები.txt",
            "ქართული ტექსტი\r\n".as_bytes(),
        ),
        ("Contracts\\ხელშეკრულება.pdf", PDF),
        ("Outlook\\.keep", b""),
        ("ხარჯთაღრიცხვა 2026.pdf", PDF),
        (
            "შენიშვნები.txt",
            "Jerry spike: ქართული ტექსტი\r\n".as_bytes(),
        ),
        ("Report Q3 (final) [v2].pdf", PDF),
    ]
}

/// Creates `root` and any missing seed file. Existing files are left alone.
pub fn ensure(root: &Path) -> io::Result<()> {
    for (relative, content) in seeds() {
        let path = root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        if !path.exists() {
            fs::write(&path, content)?;
        }
    }
    Ok(())
}

/// Lists `dir`: folders first, then files, each by name (no natural sort yet). Hidden
/// `.jerrypart` files and the `.keep` marker are left out.
pub fn list(dir: &Path) -> io::Result<Vec<Entry>> {
    let mut entries = Vec::new();
    for item in fs::read_dir(dir)? {
        let item = item?;
        let name = item.file_name();
        let text = name.to_string_lossy();
        if text == ".keep" || text.ends_with(PART_SUFFIX) {
            continue;
        }
        let meta = item.metadata()?;
        entries.push(Entry {
            path: item.path(),
            name,
            is_dir: meta.is_dir(),
            size: if meta.is_dir() { 0 } else { meta.len() },
        });
    }
    entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.cmp(&b.name)));
    Ok(entries)
}

/// Every folder under `root` (root first, then depth-first), at most `depth` levels down.
pub fn folders(root: &Path, depth: usize) -> io::Result<Vec<(PathBuf, usize)>> {
    let mut out = vec![(root.to_path_buf(), 0)];
    let mut index = 0;
    while index < out.len() {
        let (dir, level) = out[index].clone();
        index += 1;
        if level >= depth {
            continue;
        }
        let children: Vec<_> = list(&dir)?
            .into_iter()
            .filter(|entry| entry.is_dir)
            .map(|entry| (entry.path, level + 1))
            .collect();
        // Depth-first order: children right after their parent.
        for (offset, child) in children.into_iter().enumerate() {
            out.insert(index + offset, child);
        }
    }
    Ok(out)
}

/// The suffix of a file that is still being written (Data safety rule 2).
pub const PART_SUFFIX: &str = ".jerrypart";

/// A name the source gave a virtual file, made safe to create under a folder: relative, no
/// `..`, no drive or root, and characters Windows rejects replaced with `_`. `None` when nothing
/// is left. The same rule as ZIP extraction.
pub fn safe_relative(name: &str) -> Option<PathBuf> {
    let mut path = PathBuf::new();
    for part in name.split(['\\', '/']) {
        if part.trim() == ".." {
            return None;
        }
        // Windows drops trailing dots and spaces, so "x. " and "x" are the same name.
        let part = part.trim_end_matches([' ', '.']);
        if part.is_empty() {
            continue;
        }
        let clean: String = part
            .chars()
            .map(|c| {
                if c < ' ' || matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*') {
                    '_'
                } else {
                    c
                }
            })
            .collect();
        path.push(clean);
    }
    let first = path.components().next()?;
    matches!(first, Component::Normal(_)).then_some(path)
}

/// `dir\name`, or `dir\name (2).ext`, `(3)`… when the name is taken (the way Explorer names a
/// copy that would collide is "name - Copy"; the spike only needs a free name).
pub fn free_name(dir: &Path, name: &Path) -> PathBuf {
    let first = dir.join(name);
    if !exists(&first) {
        return first;
    }
    let stem = name
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let ext = name
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    let parent = first.parent().map(Path::to_path_buf).unwrap_or_default();
    (2..)
        .map(|n| parent.join(format!("{stem} ({n}){ext}")))
        .find(|candidate| !exists(candidate))
        .unwrap_or(first)
}

fn exists(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok() || fs::symlink_metadata(part_path(path)).is_ok()
}

/// The hidden `.jerrypart` file `final_path` is written as.
pub fn part_path(final_path: &Path) -> PathBuf {
    let mut name = final_path.as_os_str().to_owned();
    name.push(PART_SUFFIX);
    PathBuf::from(name)
}

const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;

/// Creates the hidden part file for `final_path`; fails if it exists.
pub fn create_part(final_path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .attributes(FILE_ATTRIBUTE_HIDDEN)
        .open(part_path(final_path))
}

/// Gives a complete part file its final name and clears the hidden attribute it was written
/// with. Fails rather than replace a file that took the name meanwhile.
pub fn commit_part(final_path: &Path) -> io::Result<()> {
    let part = part_path(final_path);
    if fs::symlink_metadata(final_path).is_ok() {
        return Err(io::Error::from(io::ErrorKind::AlreadyExists));
    }
    fs::rename(&part, final_path)?;
    clear_hidden(final_path)
}

fn clear_hidden(path: &Path) -> io::Result<()> {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetFileAttributesW(name: *const u16) -> u32;
        fn SetFileAttributesW(name: *const u16, attributes: u32) -> i32;
    }
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    // SAFETY: `wide` is a NUL-terminated path that outlives both calls.
    let ok = unsafe {
        let attributes = GetFileAttributesW(wide.as_ptr());
        attributes != u32::MAX
            && SetFileAttributesW(wide.as_ptr(), attributes & !FILE_ATTRIBUTE_HIDDEN) != 0
    };
    if ok {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

/// Copies `source` (a file or a folder tree) into `dir` under a free name, each file through a
/// hidden `.jerrypart`. Returns the new top-level path and the bytes copied. Never touches
/// `source`.
pub fn copy_into(source: &Path, dir: &Path) -> io::Result<(PathBuf, u64)> {
    let name = source
        .file_name()
        .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidInput))?;
    let target = free_name(dir, Path::new(name));
    let bytes = copy_tree(source, &target)?;
    Ok((target, bytes))
}

fn copy_tree(source: &Path, target: &Path) -> io::Result<u64> {
    let meta = fs::symlink_metadata(source)?;
    if meta.is_dir() {
        fs::create_dir(target)?;
        let mut total = 0;
        for item in fs::read_dir(source)? {
            let item = item?;
            total += copy_tree(&item.path(), &target.join(item.file_name()))?;
        }
        Ok(total)
    } else {
        let mut from = File::open(source)?;
        let mut part = create_part(target)?;
        let bytes = io::copy(&mut from, &mut part)?;
        part.sync_all()?;
        drop(part);
        commit_part(target)?;
        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ensure_seeds_once_and_list_shows_user_files() {
        let dir = tempfile::tempdir().unwrap();
        ensure(dir.path()).unwrap();
        fs::write(dir.path().join("შენიშვნები.txt"), b"edited").unwrap();
        fs::write(dir.path().join("real.dwg"), b"AC1032").unwrap();
        ensure(dir.path()).unwrap();

        let entries = list(dir.path()).unwrap();
        let names: Vec<_> = entries
            .iter()
            .map(|e| e.name.to_string_lossy().into_owned())
            .collect();
        assert_eq!(names[..3], ["Contracts", "Drawings", "Outlook"]);
        assert!(entries[0].is_dir);
        assert!(names.contains(&"real.dwg".to_owned()));
        assert_eq!(
            fs::read(dir.path().join("შენიშვნები.txt")).unwrap(),
            b"edited"
        );
        // The marker that keeps Outlook\ in the seed isn't listed.
        assert!(list(&dir.path().join("Outlook")).unwrap().is_empty());
    }

    #[test]
    fn folders_are_depth_first() {
        let dir = tempfile::tempdir().unwrap();
        ensure(dir.path()).unwrap();
        let names: Vec<_> = folders(dir.path(), 3)
            .unwrap()
            .into_iter()
            .map(|(path, depth)| {
                let name = path.strip_prefix(dir.path()).unwrap().display().to_string();
                (name, depth)
            })
            .collect();
        assert_eq!(
            names,
            [
                (String::new(), 0),
                ("Contracts".into(), 1),
                ("Drawings".into(), 1),
                ("Drawings\\2026".into(), 2),
                ("Outlook".into(), 1),
            ]
        );
    }

    #[test]
    fn safe_relative_blocks_escapes() {
        assert_eq!(safe_relative("a.pdf"), Some(PathBuf::from("a.pdf")));
        assert_eq!(
            safe_relative("ფოლდერი\\ფაილი.txt"),
            Some(PathBuf::from("ფოლდერი\\ფაილი.txt"))
        );
        assert_eq!(safe_relative("..\\evil.exe"), None);
        assert_eq!(safe_relative("a\\..\\..\\evil.exe"), None);
        assert_eq!(
            safe_relative("\\\\server\\share\\x"),
            Some(PathBuf::from("server\\share\\x"))
        );
        assert_eq!(
            safe_relative("C:\\Windows\\x"),
            Some(PathBuf::from("C_\\Windows\\x"))
        );
        assert_eq!(
            safe_relative("Re: offer? \"final\".msg"),
            Some(PathBuf::from("Re_ offer_ _final_.msg"))
        );
        assert_eq!(safe_relative("trailing. "), Some(PathBuf::from("trailing")));
        assert_eq!(safe_relative(".. "), None);
        assert_eq!(safe_relative(""), None);
    }

    #[test]
    fn copy_into_never_replaces_and_leaves_no_part() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("src");
        fs::create_dir_all(source.join("ქვე")).unwrap();
        fs::write(source.join("ქვე").join("ფაილი.txt"), b"data").unwrap();
        let file = dir.path().join("ნახაზი.dwg");
        fs::write(&file, b"AC1032").unwrap();
        let target = dir.path().join("target");
        fs::create_dir(&target).unwrap();
        fs::write(target.join("ნახაზი.dwg"), b"older").unwrap();

        let (copied, bytes) = copy_into(&file, &target).unwrap();
        assert_eq!(copied, target.join("ნახაზი (2).dwg"));
        assert_eq!(bytes, 6);
        assert_eq!(fs::read(target.join("ნახაზი.dwg")).unwrap(), b"older");
        let (tree, _) = copy_into(&source, &target).unwrap();
        assert_eq!(
            fs::read(tree.join("ქვე").join("ფაილი.txt")).unwrap(),
            b"data"
        );
        // Sources untouched, no .jerrypart left, and the copy isn't hidden.
        assert!(file.exists() && source.join("ქვე").join("ფაილი.txt").exists());
        assert!(
            list(&target)
                .unwrap()
                .iter()
                .all(|e| !e.name.to_string_lossy().ends_with(PART_SUFFIX))
        );
        assert_eq!(fs::read_dir(&target).unwrap().count(), 3);
        let attributes =
            std::os::windows::fs::MetadataExt::file_attributes(&fs::metadata(&copied).unwrap());
        assert_eq!(attributes & FILE_ATTRIBUTE_HIDDEN, 0);
    }
}
