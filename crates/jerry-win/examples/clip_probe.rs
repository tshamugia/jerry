//! Spike #15 end-to-end probe: the clipboard between Jerry and the real Explorer.
//!
//! This process plays Jerry: `jerry_win::clipboard` on an OLE STA that pumps messages, as
//! Jerry's main thread does. Explorer's side runs in explorer.exe: `explorer_verb.ps1` drives an
//! Explorer window through Shell automation, which runs Explorer's own Copy, Cut and Paste in
//! its process (the code behind Ctrl+C, Ctrl+X and Ctrl+V).
//!
//! It overwrites the clipboard and opens an Explorer window on a temp folder, so it isn't a
//! test; run it by hand: `cargo run -p jerry-win --example clip_probe`.

#![allow(clippy::print_stdout, clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use jerry_win::clipboard::{self, Op};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, MSG, PM_REMOVE, PeekMessageW, TranslateMessage,
};

const SCRIPT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/examples/explorer_verb.ps1");

type Outcome = Result<(bool, String), String>;

fn main() {
    clipboard::init_ole().expect("OleInitialize");
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("child") {
        child(&args[1..]);
        return;
    }

    let root = std::env::temp_dir().join("jerry-spike-15-probe");
    let _ = fs::remove_dir_all(&root);
    let jerry = root.join("jerry");
    let explorer = root.join("explorer");
    fs::create_dir_all(&jerry).unwrap();
    fs::create_dir_all(&explorer).unwrap();
    // explorer.exe hands the window to the running Explorer and exits at once.
    let _ = Command::new("explorer.exe").arg(&explorer).status();

    type Case<'a> = (&'a str, Box<dyn Fn() -> Outcome + 'a>);
    let (j, e) = (jerry.as_path(), explorer.as_path());
    let cases: Vec<Case> = vec![
        (
            "1. Jerry Copy -> Explorer Paste",
            Box::new(|| jerry_to_explorer(j, e, Op::Copy)),
        ),
        (
            "2. Jerry Cut -> Explorer Paste",
            Box::new(|| jerry_to_explorer(j, e, Op::Cut)),
        ),
        (
            "3. Explorer Copy -> Jerry Paste",
            Box::new(|| explorer_to_jerry(j, e, Op::Copy)),
        ),
        (
            "4. Explorer Cut -> Jerry Paste",
            Box::new(|| explorer_to_jerry(j, e, Op::Cut)),
        ),
        (
            "5. Jerry Copy, exit with flush -> Explorer Paste",
            Box::new(|| exit_then_paste(j, e, Op::Copy, true)),
        ),
        (
            "6. Jerry Cut, exit with flush -> Explorer Paste",
            Box::new(|| exit_then_paste(j, e, Op::Cut, true)),
        ),
        (
            "7. Jerry Copy, exit without flush",
            Box::new(|| exit_then_paste(j, e, Op::Copy, false)),
        ),
        (
            "8. Long path: Jerry Copy -> Jerry and Explorer Paste",
            Box::new(|| long_path(j, e)),
        ),
    ];
    // `clip_probe 2 4` runs only cases 2 and 4.
    let results: Vec<(&str, Outcome)> = cases
        .iter()
        .enumerate()
        .filter(|(index, _)| args.is_empty() || args.contains(&(index + 1).to_string()))
        .map(|(_, (name, run))| (*name, run()))
        .collect();
    let _ = explorer_verb("close", &explorer, &[]);

    println!();
    for (name, outcome) in &results {
        match outcome {
            Ok((pass, detail)) => {
                println!(
                    "{} {name}\n    {detail}",
                    if *pass { "PASS" } else { "FAIL" }
                );
            }
            Err(error) => println!("ERROR {name}\n    {error}"),
        }
    }
}

/// Jerry puts files on the clipboard; Explorer pastes them.
fn jerry_to_explorer(jerry: &Path, explorer: &Path, op: Op) -> Outcome {
    let names: &[&str] = match op {
        Op::Copy => &[
            "ნახაზი — სართული 1.dwg",
            "ხარჯთაღრიცხვა 2026.pdf",
            "საქაღალდე",
        ],
        Op::Cut => &[
            "გადასატანი.txt",
            "Report Q3 (final) [v2].pdf",
            "გადასატანი საქაღალდე",
        ],
    };
    let paths = seed(jerry, names);
    let start = Instant::now();
    let data = clipboard::set_files(&paths, op).map_err(|e| e.to_string())?;
    let set_ms = ms(start);
    let seen = explorer_verb("formats", explorer, &[])?;

    explorer_verb("paste", explorer, &[])?;
    let done = pump_until(Duration::from_secs(20), || {
        names.iter().all(|name| {
            explorer.join(name).exists() && (op == Op::Copy || !jerry.join(name).exists())
        })
    });
    // Give Explorer a moment to write back into the data object.
    pump_for(Duration::from_millis(1000));
    let sources_left = paths.iter().filter(|path| path.exists()).count();
    let inner = explorer.join(names[2]).join("შიგნით.txt").exists();
    let feedback = clipboard::feedback(&data);
    let current = clipboard::is_current(&data);
    let after = explorer_verb("formats", explorer, &[])?;
    let expected_left = if op == Op::Copy { paths.len() } else { 0 };
    Ok((
        done && inner && sources_left == expected_left,
        format!(
            "set_files {set_ms:.1} ms; another process sees: {seen}\n    \
             pasted: {done}, folder content: {inner}, sources left {sources_left}/{}; \
             written back: {feedback:?}\n    clipboard still Jerry's: {current}, owner pid \
             {:?}, formats now: [{after}]",
            paths.len(),
            clipboard::owner_process()
        ),
    ))
}

/// Explorer puts files on the clipboard; Jerry reads and pastes them.
fn explorer_to_jerry(jerry: &Path, explorer: &Path, op: Op) -> Outcome {
    let names: &[&str] = match op {
        Op::Copy => &["ექსპლორერიდან.txt", "ფოლდერი Explorer-იდან"],
        Op::Cut => &["ამოსაჭრელი.dwg", "ამოსაჭრელი საქაღალდე"],
    };
    let paths = seed(explorer, names);
    explorer_verb(verb(op), explorer, names)?;
    // Explorer lists the focused item first; compare as sets.
    let same = |found: &[PathBuf]| {
        let (mut a, mut b) = (found.to_vec(), paths.clone());
        a.sort();
        b.sort();
        a == b
    };
    let mut files = None;
    let mut last = String::from("nothing read");
    pump_until(Duration::from_secs(10), || {
        files = match clipboard::get_files() {
            Ok(Some(found)) if same(&found.paths) => Some(found),
            Ok(Some(found)) => {
                last = format!("other files: {:?}", found.paths);
                None
            }
            Ok(None) => {
                last = "no files".into();
                None
            }
            Err(error) => {
                last = format!("error {error}");
                None
            }
        };
        files.is_some()
    });
    let Some(files) = files else {
        return Err(format!(
            "Explorer's files never showed up on the clipboard; {last}"
        ));
    };
    let owner = clipboard::owner_process();
    let start = Instant::now();
    let again = clipboard::get_files().map_err(|e| e.to_string())?;
    let get_ms = ms(start);
    drop(again);

    paste_into(&files.paths, jerry, files.op).map_err(|e| e.to_string())?;
    if files.op == Op::Cut {
        clipboard::finish_cut(&files).map_err(|e| e.to_string())?;
    }
    let (read_op, effect) = (files.op, files.preferred_effect);
    drop(files);
    pump_for(Duration::from_millis(1500));

    let arrived = names.iter().all(|name| jerry.join(name).exists());
    let inner = jerry.join(names[1]).join("შიგნით.txt").exists();
    let sources_left = paths.iter().filter(|path| path.exists()).count();
    let after = match clipboard::get_files() {
        Ok(Some(files)) => format!("still {} file(s), {:?}", files.paths.len(), files.op),
        Ok(None) => "no files".into(),
        Err(error) => format!("unreadable ({error})"),
    };
    let expected_left = if op == Op::Copy { paths.len() } else { 0 };
    // After a pasted cut the clipboard is emptied, as Explorer does (finish_cut).
    let cleared = op == Op::Copy || after == "no files";
    Ok((
        read_op == op && arrived && inner && sources_left == expected_left && cleared,
        format!(
            "owner pid {owner:?} (probe {}); Preferred DropEffect {effect:?} -> {read_op:?}; \
             get_files {get_ms:.1} ms\n    pasted: {arrived}, folder content: {inner}, \
             sources left {sources_left}/{}; clipboard afterwards: {after}",
            clipboard::current_process(),
            paths.len()
        ),
    ))
}

/// A child "Jerry" copies or cuts and exits, with or without OleFlushClipboard; then Explorer
/// pastes. With the flush every format stays. Without it CF_HDROP is always gone, and the rest
/// depends on timing: formats some other reader (clipboard history, say) already rendered stay,
/// so the clipboard is either empty or holds only Shell IDList Array and the effect, which
/// Explorer can paste but CF_HDROP readers (Outlook, Telegram, Jerry) can't.
fn exit_then_paste(jerry: &Path, explorer: &Path, op: Op, flush: bool) -> Outcome {
    let name = match (op, flush) {
        (Op::Copy, true) => "დახურვის შემდეგ ასლი.txt",
        (Op::Cut, true) => "დახურვის შემდეგ გადატანა.txt",
        _ => "flush-ის გარეშე.txt",
    };
    let path = seed(jerry, &[name]).remove(0);
    let child = Command::new(std::env::current_exe().map_err(|e| e.to_string())?)
        .args(["child", verb(op), if flush { "flush" } else { "noflush" }])
        .arg(&path)
        .output()
        .map_err(|e| e.to_string())?;
    let said = String::from_utf8_lossy(&child.stdout).trim().to_owned();

    let files = clipboard::get_files();
    let has_hdrop = matches!(files, Ok(Some(_)));
    let on_clipboard = match files {
        Ok(Some(files)) => format!(
            "{} file(s), Preferred DropEffect {:?} -> {:?}",
            files.paths.len(),
            files.preferred_effect,
            files.op
        ),
        Ok(None) => "no CF_HDROP".into(),
        Err(error) => format!("unreadable ({error})"),
    };
    let seen = explorer_verb("formats", explorer, &[])?;
    explorer_verb("paste", explorer, &[])?;
    let wait = if flush { 20 } else { 5 };
    let pasted = pump_until(Duration::from_secs(wait), || {
        explorer.join(name).exists() && (op == Op::Copy || !path.exists())
    });
    Ok((
        if flush {
            pasted && has_hdrop
        } else {
            !has_hdrop
        },
        format!(
            "child: {said}; after it exited: {on_clipboard}; another process sees: {seen}\n    \
             Explorer pasted: {pasted}, source still there: {}",
            path.exists()
        ),
    ))
}

/// A file whose path is longer than MAX_PATH, given as core would (`\\?\`).
fn long_path(jerry: &Path, explorer: &Path) -> Outcome {
    let mut dir = jerry.join("გრძელი");
    while dir.as_os_str().len() < 300 {
        dir.push("ძალიან გრძელი საქაღალდის სახელი");
    }
    let verbatim = PathBuf::from(format!(r"\\?\{}", dir.display()));
    fs::create_dir_all(&verbatim).unwrap();
    let name = "გრძელი გზის ნახაზი.dwg";
    fs::write(verbatim.join(name), b"AC1032").unwrap();
    let plain = dir.join(name);

    let data = clipboard::set_files(&[verbatim.join(name)], Op::Copy).map_err(|e| e.to_string())?;
    let formats = clipboard::format_names(&data).map_err(|e| e.to_string())?;
    let read = clipboard::get_files().map_err(|e| e.to_string())?;
    let read_ok = read
        .as_ref()
        .is_some_and(|files| files.paths == [plain.clone()]);
    let target = jerry.join("long-paste");
    fs::create_dir_all(&target).unwrap();
    if let Some(files) = &read {
        paste_into(&files.paths, &target, files.op).map_err(|e| e.to_string())?;
    }
    let jerry_pasted = target.join(name).exists();

    explorer_verb("paste", explorer, &[])?;
    let explorer_pasted = pump_until(Duration::from_secs(10), || explorer.join(name).exists());
    Ok((
        read_ok && jerry_pasted,
        format!(
            "{} chars; formats: {formats:?}\n    CF_HDROP read back exactly: {read_ok}; \
             Jerry pasted: {jerry_pasted}; Explorer pasted: {explorer_pasted}",
            plain.as_os_str().len()
        ),
    ))
}

/// `clip_probe child <copy|cut> <flush|noflush> <path>`: a short-lived Jerry.
fn child(args: &[String]) {
    let op = if args[0] == "cut" { Op::Cut } else { Op::Copy };
    let paths: Vec<PathBuf> = args[2..].iter().map(PathBuf::from).collect();
    let data = clipboard::set_files(&paths, op).expect("set_files");
    pump_for(Duration::from_millis(300));
    if args[1] == "flush" {
        let start = Instant::now();
        let flushed = clipboard::flush(&data).expect("OleFlushClipboard");
        println!("flushed {flushed} in {:.1} ms", ms(start));
    } else {
        println!("exiting without flush");
    }
}

/// Jerry's paste, reduced to what the probe needs: rename for Cut (same volume), copy for Copy.
fn paste_into(paths: &[PathBuf], target: &Path, op: Op) -> std::io::Result<()> {
    for path in paths {
        let to = target.join(path.file_name().unwrap_or_default());
        match op {
            Op::Cut => fs::rename(path, &to)?,
            Op::Copy => copy_tree(path, &to)?,
        }
    }
    Ok(())
}

fn copy_tree(from: &Path, to: &Path) -> std::io::Result<()> {
    if from.is_dir() {
        fs::create_dir_all(to)?;
        for entry in fs::read_dir(from)? {
            let entry = entry?;
            copy_tree(&entry.path(), &to.join(entry.file_name()))?;
        }
        Ok(())
    } else {
        fs::copy(from, to).map(|_| ())
    }
}

/// Creates the named items in `dir`: a name with a dot is a file, one without is a folder with
/// a file inside.
fn seed(dir: &Path, names: &[&str]) -> Vec<PathBuf> {
    names
        .iter()
        .map(|name| {
            let path = dir.join(name);
            if name.contains('.') {
                fs::write(&path, format!("Jerry spike #15: {name}\r\n")).unwrap();
            } else {
                fs::create_dir_all(&path).unwrap();
                fs::write(path.join("შიგნით.txt"), "inside\r\n").unwrap();
            }
            path
        })
        .collect()
}

fn verb(op: Op) -> &'static str {
    match op {
        Op::Copy => "copy",
        Op::Cut => "cut",
    }
}

/// Runs a verb in Explorer (see explorer_verb.ps1), pumping messages meanwhile: Explorer calls
/// back into Jerry's data object while it pastes.
fn explorer_verb(verb: &str, dir: &Path, names: &[&str]) -> Result<String, String> {
    let mut child = Command::new("powershell")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
            SCRIPT,
        ])
        .env("JERRY_VERB", verb)
        .env("JERRY_DIR", dir)
        .env("JERRY_NAMES", names.join("|"))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    pump_until(Duration::from_secs(60), || {
        matches!(child.try_wait(), Ok(Some(_)))
    });
    let output = child.wait_with_output().map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(format!(
            "explorer_verb {verb}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

/// Pumps this thread's messages until `done` or the timeout; returns whether `done` held.
fn pump_until(timeout: Duration, mut done: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        pump();
        if done() {
            return true;
        }
        if Instant::now() > deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn pump_for(duration: Duration) {
    pump_until(duration, || false);
}

fn pump() {
    let mut msg = MSG::default();
    // SAFETY: a valid MSG buffer; messages for any of this thread's windows.
    while unsafe { PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE) }.as_bool() {
        // SAFETY: a message PeekMessageW just filled in.
        unsafe {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

fn ms(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1000.0
}
