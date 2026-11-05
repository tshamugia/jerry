//! The address bar's text on temp folders: every form of a path leads to the right folder, files
//! open their folder, and partly typed paths complete.

// Test code throughout, helpers included (clippy allows `expect` in tests).
#![cfg(test)]

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use jerry_core::ErrorCode;
use jerry_core::address::{Context, Resolved, clean, complete, expand, resolve, subfolders};
use jerry_core::path::long_path;
use tempfile::TempDir;

/// `პროექტები` with `ნახაზები`, `Plans 10`, `Plans 2`, hidden `.cache`, and `ნახაზი.dwg` in it.
fn work() -> (TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("temp dir");
    let work = dir.path().join("პროექტები");
    for sub in ["ნახაზები", "Plans 10", "Plans 2", ".cache"] {
        std::fs::create_dir_all(work.join(sub)).expect("mkdir");
    }
    std::fs::write(work.join("ნახაზი.dwg"), b"dwg").expect("write");
    hide(&work.join(".cache"));
    (dir, work)
}

fn hide(path: &Path) {
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::Storage::FileSystem::{FILE_ATTRIBUTE_HIDDEN, SetFileAttributesW};
    use windows::core::PCWSTR;
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
    // SAFETY: a NUL-terminated path to a folder this test made.
    unsafe { SetFileAttributesW(PCWSTR(wide.as_ptr()), FILE_ATTRIBUTE_HIDDEN) }.expect("hide");
}

/// A context with `WORK` as a variable and `Downloads` as a known folder, both `work`.
fn context<'a>(
    base: Option<&'a Path>,
    known: &'a dyn Fn(&str) -> Option<PathBuf>,
    var: &'a dyn Fn(&str) -> Option<OsString>,
) -> Context<'a> {
    Context { base, known, var }
}

fn long(path: &Path) -> PathBuf {
    long_path(path).expect("long path")
}

#[test]
fn text_is_cleaned_of_quotes_and_file_urls() {
    assert_eq!(clean(r#"  "C:\a b\c"  "#), r"C:\a b\c");
    assert_eq!(clean("file:///C:/a%20b/%E1%83%9C"), r"C:\a b\ნ");
    assert_eq!(clean("FILE://server/share/x"), r"\\server\share\x");
    assert_eq!(clean(r"D:\100%"), r"D:\100%");
}

#[test]
fn variables_expand_and_unknown_ones_stay() {
    let var = |name: &str| (name == "APPDATA").then(|| OsString::from(r"C:\Roaming"));
    assert_eq!(
        expand(r"%APPDATA%\Jerry", &var),
        OsString::from(r"C:\Roaming\Jerry")
    );
    assert_eq!(
        expand(r"%NOPE%\x 50%", &var),
        OsString::from(r"%NOPE%\x 50%")
    );
}

#[test]
fn absolute_relative_variable_and_known_paths_lead_to_their_folder() {
    let (_dir, work) = work();
    let parent = work.parent().expect("parent").to_path_buf();
    let known_work = work.clone();
    let known = move |name: &str| {
        name.eq_ignore_ascii_case("downloads")
            .then(|| known_work.clone())
    };
    let var_work = work.clone();
    let var = move |name: &str| (name == "WORK").then(|| var_work.clone().into_os_string());
    let shown = context(Some(&work), &known, &var);
    let folder = |text: &str| resolve(text, &shown).expect(text).folder;

    let drawings = long(&work.join("ნახაზები"));
    assert_eq!(folder(&work.join("ნახაზები").to_string_lossy()), drawings);
    assert_eq!(
        folder(&format!("\"{}\\ნახაზები\"", work.display())),
        drawings
    );
    assert_eq!(folder("ნახაზები"), drawings);
    assert_eq!(folder(r"..\პროექტები\ნახაზები"), drawings);
    assert_eq!(folder(r"%WORK%\ნახაზები"), drawings);
    assert_eq!(folder(r"downloads\ნახაზები"), drawings);
    assert_eq!(folder("Downloads"), long(&work));
    assert_eq!(folder(".."), long(&parent));
}

#[test]
fn a_subfolder_wins_over_a_known_folder_of_the_same_name() {
    let (_dir, work) = work();
    std::fs::create_dir(work.join("Downloads")).expect("mkdir");
    let elsewhere = work.join("Plans 2");
    let known = move |_: &str| Some(elsewhere.clone());
    let var = |_: &str| None;
    let resolved = resolve("Downloads", &context(Some(&work), &known, &var)).expect("resolves");
    assert_eq!(resolved.folder, long(&work.join("Downloads")));
}

#[test]
fn a_file_opens_its_folder_and_is_selected() {
    let (_dir, work) = work();
    let none = |_: &str| None;
    let var = |_: &str| None;
    let text = work.join("ნახაზი.dwg").to_string_lossy().into_owned();
    assert_eq!(
        resolve(&text, &context(None, &none, &var)).expect("resolves"),
        Resolved {
            folder: long(&work),
            select: Some(OsString::from("ნახაზი.dwg")),
        }
    );
}

#[test]
fn a_drive_alone_is_its_root() {
    let none = |_: &str| None;
    let var = |_: &str| None;
    let resolved = resolve("c:", &context(None, &none, &var)).expect("C: is there");
    assert_eq!(resolved.folder, long(Path::new(r"c:\")));
}

#[test]
fn what_isnt_there_fails_without_a_folder() {
    let (_dir, work) = work();
    let none = |_: &str| None;
    let var = |_: &str| None;
    let shown = context(Some(&work), &none, &var);
    let error = resolve("აღარ არის", &shown).expect_err("missing");
    assert_eq!(error.code, ErrorCode::NotFound);
    assert_eq!(error.path, Some(long(&work.join("აღარ არის"))));
    let error = resolve("  ", &shown).expect_err("empty");
    assert_eq!(error.code, ErrorCode::InvalidName);
    // Relative text with no folder shown names nothing.
    let error = resolve("x", &context(None, &none, &var)).expect_err("nowhere");
    assert_eq!(error.code, ErrorCode::NotFound);
}

#[test]
fn subfolders_are_in_natural_order_hidden_ones_on_request() {
    let (_dir, work) = work();
    let names = |show_hidden| -> Vec<String> {
        subfolders(&work, show_hidden)
            .expect("lists")
            .into_iter()
            .map(|name| name.to_string_lossy().into_owned())
            .collect()
    };
    assert_eq!(names(false), ["Plans 2", "Plans 10", "ნახაზები"]);
    assert_eq!(names(true), [".cache", "Plans 2", "Plans 10", "ნახაზები"]);
}

#[test]
fn partly_typed_paths_complete_as_typed() {
    let (_dir, work) = work();
    let var_work = work.clone();
    let var = move |name: &str| (name == "WORK").then(|| var_work.clone().into_os_string());
    let none = |_: &str| None;
    let shown = context(Some(&work), &none, &var);
    let known = ["Desktop", "Downloads"];

    let typed = format!(r"{}\pl", work.display());
    assert_eq!(
        complete(&typed, &shown, &known, false, 10),
        [
            format!(r"{}\Plans 2", work.display()),
            format!(r"{}\Plans 10", work.display())
        ]
    );
    assert_eq!(
        complete(r"%WORK%\ნა", &shown, &known, false, 10),
        [r"%WORK%\ნახაზები"]
    );
    assert_eq!(
        complete("d", &shown, &known, false, 10),
        ["Desktop", "Downloads"]
    );
    assert_eq!(complete("p", &shown, &known, false, 1), ["Plans 2"]);
    assert_eq!(
        complete(r"%WORK%\.c", &shown, &known, true, 10),
        [r"%WORK%\.cache"]
    );
    assert!(complete(r"Z:\nowhere\x", &shown, &known, false, 10).is_empty());
}
