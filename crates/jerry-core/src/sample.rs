//! Spike #13 only: a fixed sample folder to drag files out of. Real listing is #24.
//!
//! The folder lives under the user's temp dir. Seed files are created when missing and never
//! overwritten, and anything the user adds (a real DWG, say) is listed too.

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

/// A minimal valid one-page PDF, so PDF targets (Outlook preview, Chrome, Telegram) open it.
const PDF: &[u8] = b"%PDF-1.4
1 0 obj<</Type/Catalog/Pages 2 0 R>>endobj
2 0 obj<</Type/Pages/Kids[3 0 R]/Count 1>>endobj
3 0 obj<</Type/Page/Parent 2 0 R/MediaBox[0 0 300 120]/Contents 4 0 R/Resources<</Font<</F1 5 0 R>>>>>>endobj
4 0 obj<</Length 45>>stream
BT /F1 18 Tf 30 60 Td (Jerry spike #13) Tj ET
endstream endobj
5 0 obj<</Type/Font/Subtype/Type1/BaseFont/Helvetica>>endobj
trailer<</Root 1 0 R>>
%%EOF
";

/// Seed files: Georgian and plain names, a DWG-like and a PDF-like file, a folder.
fn seeds() -> [(&'static str, &'static [u8]); 5] {
    [
        // Only the DWG signature: AutoCAD sees a DWG and reports it as damaged, which still
        // proves the file arrived. Copy a real drawing into the folder to test opening.
        ("ნახაზი — სართული 1.dwg", b"AC1032\0\0\0\0\0\0"),
        ("ხარჯთაღრიცხვა 2026.pdf", PDF),
        (
            "შენიშვნები.txt",
            "Jerry spike #13: ქართული ტექსტი\r\n".as_bytes(),
        ),
        ("Report Q3 (final) [v2].pdf", PDF),
        ("საქაღალდე\\შიგნით.txt", b"inside a folder\r\n"),
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
        assert_eq!(names[0], "საქაღალდე");
        assert!(entries[0].is_dir);
        assert!(names.contains(&"real.dwg".to_owned()));
        assert_eq!(
            fs::read(dir.path().join("შენიშვნები.txt")).unwrap(),
            b"edited"
        );
    }
}
