//! Dragging files out of Jerry with Windows OLE (Tech PRD «Drag & drop» → «გადათრევა Jerry-დან»,
//! spike #13).
//!
//! Files leave Jerry as the Shell's own data object, the one Explorer drags ([`data_object`]), so
//! every target (Explorer, the desktop, Outlook, Telegram, browsers, AutoCAD) gets exactly what a
//! drag from Explorer gives it. The drag loop, the drag image and the count badge are Windows' own
//! (`SHDoDragDrop` with its default drop source). The target copies or moves: Jerry's transfer
//! queue takes no part, and Jerry deletes nothing afterwards (see [`drag_files`]).
//!
//! Main thread only: OLE drag & drop needs the OLE-initialized STA thread that pumps Jerry's
//! window messages ([`main_thread::init`]). The drag starts from an `async` command through
//! [`run_on_main`], never inside a WebView2 or tao callback (ADR-0002).
//!
//! [`data_object`]: crate::data_object
//! [`main_thread::init`]: crate::main_thread::init
//! [`run_on_main`]: crate::main_thread::run_on_main

use std::cell::Cell;
use std::path::PathBuf;
use std::time::Instant;

use jerry_core::JerryError;
use windows::Win32::Foundation::{DRAGDROP_S_DROP, HWND};
use windows::Win32::System::Ole::{
    DROPEFFECT, DROPEFFECT_COPY, DROPEFFECT_LINK, DROPEFFECT_MOVE, DROPEFFECT_NONE,
};
use windows::Win32::UI::HiDpi::GetSystemMetricsForDpi;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON, VK_RBUTTON};
use windows::Win32::UI::Shell::CFSTR_PERFORMEDDROPEFFECT;
use windows::Win32::UI::WindowsAndMessaging::{SM_CXDRAG, SM_CYDRAG};
use windows::core::{HRESULT, Interface};

use crate::data_object;

/// What a drop target did with the files.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DropEffect {
    None,
    Copy,
    Move,
    Link,
}

impl DropEffect {
    fn from_raw(effect: u32) -> Self {
        // A target reports one effect; MOVE wins if it ever reports several.
        if effect & DROPEFFECT_MOVE.0 != 0 {
            Self::Move
        } else if effect & DROPEFFECT_COPY.0 != 0 {
            Self::Copy
        } else if effect & DROPEFFECT_LINK.0 != 0 {
            Self::Link
        } else {
            Self::None
        }
    }
}

/// How a drag ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DragOutcome {
    /// No drag: the mouse button was up by the time the main thread got to it (a quick flick),
    /// or another drag was still running. A drag now would drop at once, onto Jerry itself.
    NotStarted,
    /// Esc, or released over nothing that takes files.
    Cancelled,
    /// Dropped on a target, which did `effect`.
    Dropped {
        effect: DropEffect,
        /// The target copied the files and asked the source to delete them ("Performed
        /// DropEffect" = MOVE, an unoptimized move). Jerry doesn't: the files stay where they were.
        unoptimized_move: bool,
    },
}

thread_local! {
    /// A drag is running on this (the main) thread: `SHDoDragDrop`'s loop dispatches Jerry's
    /// tasks, so a second drag could start inside the first.
    static DRAGGING: Cell<bool> = const { Cell::new(false) };
}

/// Windows' drag threshold (`SM_CXDRAG`, `SM_CYDRAG`) at 96 DPI, so in DIPs, which are the page's
/// CSS px: a press that moves farther than this with the button down is a drag. 4 × 4 unless the
/// user changed it.
pub fn threshold() -> (u32, u32) {
    // SAFETY: plain queries without pointers.
    let metric = |index| unsafe { GetSystemMetricsForDpi(index, USER_DEFAULT_DPI) };
    let (x, y) = (metric(SM_CXDRAG), metric(SM_CYDRAG));
    (positive(x), positive(y))
}

const USER_DEFAULT_DPI: u32 = 96;
/// Windows' own default, if the metric can't be read.
const DEFAULT_THRESHOLD: u32 = 4;

fn positive(metric: i32) -> u32 {
    u32::try_from(metric)
        .ok()
        .filter(|&px| px > 0)
        .unwrap_or(DEFAULT_THRESHOLD)
}

/// Drags `paths` (core's, `\\?\` long paths welcome) out of Jerry and returns once the user drops
/// or cancels. `owner` is Jerry's window the drag starts from; Windows draws the drag image
/// (the files' icons or thumbnails with a count badge).
///
/// Main thread only (see the module docs), while the mouse button that started the drag is down;
/// if it is up already, nothing happens ([`DragOutcome::NotStarted`]). Allows copy, move and link,
/// as Explorer does, so the target's rule applies: Explorer and the desktop move on the same drive
/// and copy to another one, Ctrl copies, Shift moves, a right-button drag asks. The target does the
/// work; after an unoptimized move (the target copied and asks the source to delete) Jerry deletes
/// nothing, since it can't verify the copy (data safety rule 1).
///
/// `NotFound` with its path for a file that is gone.
pub fn drag_files(owner: isize, paths: &[PathBuf]) -> Result<DragOutcome, JerryError> {
    if DRAGGING.get() || !button_down() {
        tracing::debug!(dragging = DRAGGING.get(), "drag not started");
        return Ok(DragOutcome::NotStarted);
    }
    let prepared = Instant::now();
    let data = data_object::files(paths)?;
    let prepare_ms = prepared.elapsed().as_millis() as u64;

    // The `windows` wrapper folds DRAGDROP_S_DROP and DRAGDROP_S_CANCEL into Ok, so call the
    // export directly to tell a drop from a cancel.
    windows::core::link!("shell32.dll" "system" fn SHDoDragDrop(
        hwnd: HWND,
        pdata: *mut core::ffi::c_void,
        pdsrc: *mut core::ffi::c_void,
        dweffect: DROPEFFECT,
        pdweffect: *mut DROPEFFECT,
    ) -> HRESULT);

    let allowed = DROPEFFECT(DROPEFFECT_COPY.0 | DROPEFFECT_MOVE.0 | DROPEFFECT_LINK.0);
    let mut effect = DROPEFFECT_NONE;
    let started = Instant::now();
    DRAGGING.set(true);
    // SAFETY: `data` is a live IDataObject of this thread, borrowed for the whole call; a null
    // drop source makes the Shell use its default one; `effect` is a valid out pointer. `owner` is
    // Jerry's window (a stale handle only costs the drag image).
    let hr = unsafe {
        SHDoDragDrop(
            HWND(owner as *mut core::ffi::c_void),
            data.as_raw(),
            core::ptr::null_mut(),
            allowed,
            &mut effect,
        )
    };
    DRAGGING.set(false);
    let drag_ms = started.elapsed().as_millis() as u64;
    hr.ok()?;

    let outcome = if hr == DRAGDROP_S_DROP {
        let performed = data_object::dword(&data, CFSTR_PERFORMEDDROPEFFECT);
        outcome(effect.0, performed)
    } else {
        DragOutcome::Cancelled
    };
    tracing::info!(
        items = paths.len(),
        ?outcome,
        prepare_ms,
        drag_ms,
        "drag finished"
    );
    if let DragOutcome::Dropped {
        unoptimized_move: true,
        ..
    } = outcome
    {
        tracing::warn!("the target asked Jerry to delete the dragged files; they stay");
    }
    Ok(outcome)
}

/// How a drop ended, from the effect `Drop` returned and the "Performed DropEffect" the target
/// wrote back, if any (MSDN «Handling Shell Data Transfer Scenarios»). After an optimized move the
/// target moved the files itself and writes NONE, or nothing; an unoptimized move writes MOVE.
fn outcome(effect: u32, performed: Option<u32>) -> DragOutcome {
    let effect = DropEffect::from_raw(effect);
    let unoptimized_move = effect == DropEffect::Move
        && performed.is_some_and(|done| DropEffect::from_raw(done) == DropEffect::Move);
    DragOutcome::Dropped {
        effect,
        unoptimized_move,
    }
}

/// Whether a mouse button that drags (left or right) is down now: the page asked for the drag a
/// few milliseconds ago, and the button may have come up since. `GetAsyncKeyState` reads the
/// physical buttons, so swapped buttons don't matter: either one counts.
fn button_down() -> bool {
    [VK_LBUTTON, VK_RBUTTON].iter().any(|key| {
        // SAFETY: a plain Win32 query without pointers.
        let state = unsafe { GetAsyncKeyState(i32::from(key.0)) };
        state < 0
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_threshold_is_windows_metric() {
        let (x, y) = threshold();
        assert!((1..=64).contains(&x), "{x}");
        assert!((1..=64).contains(&y), "{y}");
        assert_eq!(positive(0), DEFAULT_THRESHOLD);
        assert_eq!(positive(-1), DEFAULT_THRESHOLD);
        assert_eq!(positive(6), 6);
    }

    #[test]
    fn drops_report_the_effect_and_an_unoptimized_move() {
        let (copy, moved, link) = (DROPEFFECT_COPY.0, DROPEFFECT_MOVE.0, DROPEFFECT_LINK.0);
        let dropped = |effect, unoptimized_move| DragOutcome::Dropped {
            effect,
            unoptimized_move,
        };
        assert_eq!(outcome(copy, None), dropped(DropEffect::Copy, false));
        assert_eq!(outcome(link, None), dropped(DropEffect::Link, false));
        // Optimized: the target moved them and says NONE, or nothing.
        assert_eq!(outcome(moved, Some(0)), dropped(DropEffect::Move, false));
        assert_eq!(outcome(moved, None), dropped(DropEffect::Move, false));
        // Unoptimized: copied, and the source should delete. Jerry won't.
        assert_eq!(outcome(moved, Some(moved)), dropped(DropEffect::Move, true));
        assert_eq!(outcome(copy, Some(copy)), dropped(DropEffect::Copy, false));
        assert_eq!(outcome(0, None), dropped(DropEffect::None, false));
        assert_eq!(DropEffect::from_raw(copy | moved), DropEffect::Move);
    }
}
