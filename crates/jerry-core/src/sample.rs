//! Spike #15 only: a fixed sample folder to copy, cut and paste files with. Real listing is #24,
//! real transfers are the transfer engine's.
//!
//! The folder lives under the user's temp dir. Seed files are created only with the folder, so
//! files cut away stay gone (delete the folder to start over), and anything the user adds or
//! pastes is listed too.

use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// One item in the sample folder.
#[derive(Clone, Debug)]
pub struct Entry {
    pub path: PathBuf,
    pub name: OsString,
    pub is_dir: bool,
    pub size: u64,
}

/// A minimal valid one-page PDF, so PDF targets (Outlook preview, Telegram) open it.
const PDF: &[u8] = b"%PDF-1.4
1 0 obj<</Type/Catalog/Pages 2 0 R>>endobj
2 0 obj<</Type/Pages/Kids[3 0 R]/Count 1>>endobj
3 0 obj<</Type/Page/Parent 2 0 R/MediaBox[0 0 300 120]/Contents 4 0 R/Resources<</Font<</F1 5 0 R>>>>>>endobj
4 0 obj<</Length 45>>stream
BT /F1 18 Tf 30 60 Td (Jerry spike #15) Tj ET
endstream endobj
5 0 obj<</Type/Font/Subtype/Type1/BaseFont/Helvetica>>endobj
trailer<</Root 1 0 R>>
%%EOF
";

/// Seed files: Georgian and plain names, a DWG-like and a PDF file, a folder.
fn seeds() -> [(&'static str, &'static [u8]); 5] {
    [
        // Only the DWG signature: enough to show the file arrived.
        ("ნახაზი — სართული 1.dwg", b"AC1032\0\0\0\0\0\0"),
        ("ხარჯთაღრიცხვა 2026.pdf", PDF),
        (
            "შენიშვნები.txt",
            "Jerry spike #15: ქართული ტექსტი\r\n".as_bytes(),
        ),
        ("Report Q3 (final) [v2].pdf", PDF),
        ("საქაღალდე\\შიგნით.txt", b"inside a folder\r\n"),
    ]
}

/// Creates `root` with the seed files, unless it already exists.
pub fn ensure(root: &Path) -> io::Result<()> {
    if root.exists() {
        return Ok(());
    }
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

/// Lists `root`: folders first, then files, each by name (no natural sort yet).
pub fn list(root: &Path) -> io::Result<Vec<Entry>> {
    let mut entries = Vec::new();
    for item in fs::read_dir(root)? {
        let item = item?;
        let meta = item.metadata()?;
        entries.push(Entry {
            path: item.path(),
            name: item.file_name(),
            is_dir: meta.is_dir(),
            size: if meta.is_dir() { 0 } else { meta.len() },
        });
    }
    entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.cmp(&b.name)));
    Ok(entries)
}

/// What a paste did with each item.
#[derive(Debug, Default)]
pub struct Pasted {
    /// Names as they are now in the target folder.
    pub done: Vec<OsString>,
    /// Names left alone, with why.
    pub skipped: Vec<(OsString, &'static str)>,
}

/// Pastes `paths` into `target`: copies them, or moves them for a cut.
///
/// A copy never overwrites: a taken name becomes "name - Copy.ext", as in Explorer. A move
/// renames on the same volume; across volumes it copies, checks the copy and only then deletes
/// the source (data safety rule 1). Files are written as `.jerrypart` and renamed when complete
/// (rule 2). A move onto a taken name, or into the folder the item is already in, is skipped
/// (Explorer would ask; the spike doesn't).
pub fn paste(paths: &[PathBuf], target: &Path, cut: bool) -> io::Result<Pasted> {
    let mut pasted = Pasted::default();
    for path in paths {
        let Some(name) = path.file_name().map(OsString::from) else {
            pasted
                .skipped
                .push((path.as_os_str().into(), "no file name"));
            continue;
        };
        if !path.exists() {
            pasted.skipped.push((name, "gone"));
            continue;
        }
        if cut {
            if path.parent() == Some(target) {
                pasted.skipped.push((name, "already here"));
                continue;
            }
            let to = target.join(&name);
            if to.exists() {
                pasted.skipped.push((name, "name taken"));
                continue;
            }
            move_item(path, &to)?;
            pasted.done.push(name);
        } else {
            let to = free_name(target, &name);
            copy_tree(path, &to)?;
            pasted
                .done
                .push(to.file_name().map(OsString::from).unwrap_or(name));
        }
    }
    Ok(pasted)
}

/// Windows' ERROR_NOT_SAME_DEVICE: a rename can't cross volumes.
const NOT_SAME_DEVICE: i32 = 17;

fn move_item(from: &Path, to: &Path) -> io::Result<()> {
    match fs::rename(from, to) {
        Err(error) if error.raw_os_error() == Some(NOT_SAME_DEVICE) => {
            copy_tree(from, to)?;
            if size_of_tree(from)? != size_of_tree(to)? {
                return Err(io::Error::other("the copy doesn't match the source"));
            }
            if from.is_dir() {
                fs::remove_dir_all(from)
            } else {
                fs::remove_file(from)
            }
        }
        result => result,
    }
}

fn copy_tree(from: &Path, to: &Path) -> io::Result<()> {
    if from.is_dir() {
        fs::create_dir(to)?;
        for item in fs::read_dir(from)? {
            let item = item?;
            copy_tree(&item.path(), &to.join(item.file_name()))?;
        }
        Ok(())
    } else {
        let mut part = to.as_os_str().to_owned();
        part.push(".jerrypart");
        let part = PathBuf::from(part);
        fs::copy(from, &part)?;
        fs::rename(&part, to)
    }
}

fn size_of_tree(path: &Path) -> io::Result<u64> {
    if path.is_dir() {
        let mut total = 0;
        for item in fs::read_dir(path)? {
            total += size_of_tree(&item?.path())?;
        }
        Ok(total)
    } else {
        Ok(fs::metadata(path)?.len())
    }
}

/// `target\name` if free, else "stem - Copy.ext", "stem - Copy (2).ext", ... as Explorer names
/// them.
fn free_name(target: &Path, name: &OsString) -> PathBuf {
    let first = target.join(name);
    if !first.exists() {
        return first;
    }
    let as_path = Path::new(name);
    let is_dir = first.is_dir();
    let stem = if is_dir {
        Some(name.as_os_str())
    } else {
        as_path.file_stem()
    }
    .unwrap_or(name.as_os_str());
    let extension = if is_dir { None } else { as_path.extension() };
    (1..)
        .map(|n| {
            let mut candidate = stem.to_owned();
            candidate.push(" - Copy");
            if n > 1 {
                candidate.push(format!(" ({n})"));
            }
            if let Some(extension) = extension {
                candidate.push(".");
                candidate.push(extension);
            }
            target.join(candidate)
        })
        .find(|candidate| !candidate.exists())
        .unwrap_or(first)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A sample folder seeded inside a fresh temp dir (which `ensure` would leave alone).
    fn seeded() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("sample");
        ensure(&root).unwrap();
        (dir, root)
    }

    #[test]
    fn ensure_seeds_once_and_list_shows_user_files() {
        let (_dir, root) = seeded();
        fs::write(root.join("შენიშვნები.txt"), b"edited").unwrap();
        fs::write(root.join("real.dwg"), b"AC1032").unwrap();
        fs::remove_file(root.join("ხარჯთაღრიცხვა 2026.pdf")).unwrap();
        ensure(&root).unwrap();

        let entries = list(&root).unwrap();
        let names: Vec<_> = entries
            .iter()
            .map(|e| e.name.to_string_lossy().into_owned())
            .collect();
        assert_eq!(names[0], "საქაღალდე");
        assert!(entries[0].is_dir);
        assert!(names.contains(&"real.dwg".to_owned()));
        // A seed cut or deleted away stays gone.
        assert!(!names.contains(&"ხარჯთაღრიცხვა 2026.pdf".to_owned()));
        assert_eq!(fs::read(root.join("შენიშვნები.txt")).unwrap(), b"edited");
    }

    #[test]
    fn copy_never_overwrites_and_names_like_explorer() {
        let (_dir, root) = seeded();
        let file = root.join("ნახაზი — სართული 1.dwg");
        let folder = root.join("საქაღალდე");

        for _ in 0..2 {
            paste(&[file.clone(), folder.clone()], &root, false).unwrap();
        }

        for name in [
            "ნახაზი — სართული 1 - Copy.dwg",
            "ნახაზი — სართული 1 - Copy (2).dwg",
            r"საქაღალდე - Copy\შიგნით.txt",
            r"საქაღალდე - Copy (2)\შიგნით.txt",
        ] {
            assert!(root.join(name).exists(), "{name}");
        }
        assert!(file.exists() && folder.exists());
    }

    #[test]
    fn cut_moves_and_skips_what_it_must_not_touch() {
        let (_dir, from) = seeded();
        let to = tempfile::tempdir().unwrap();
        fs::write(to.path().join("შენიშვნები.txt"), b"theirs").unwrap();
        let paths =
            ["ხარჯთაღრიცხვა 2026.pdf", "საქაღალდე", "შენიშვნები.txt"].map(|name| from.join(name));

        let pasted = paste(&paths, to.path(), true).unwrap();

        assert_eq!(pasted.done.len(), 2);
        assert_eq!(pasted.skipped, [("შენიშვნები.txt".into(), "name taken")]);
        assert!(to.path().join(r"საქაღალდე\შიგნით.txt").exists());
        assert!(!paths[0].exists() && !paths[1].exists() && paths[2].exists());
        assert_eq!(
            fs::read(to.path().join("შენიშვნები.txt")).unwrap(),
            b"theirs"
        );

        let again = paste(&[to.path().join("საქაღალდე")], to.path(), true).unwrap();
        assert_eq!(again.skipped, [("საქაღალდე".into(), "already here")]);
    }
}
