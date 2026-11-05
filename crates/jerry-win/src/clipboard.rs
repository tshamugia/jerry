//! The Windows clipboard, on the main STA thread.
//!
//! For now, text: "Copy path" puts a folder's path there as `CF_UNICODETEXT`. Files (Copy, Cut,
//! Paste with Explorer) come with #38.

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::time::Duration;

use jerry_core::JerryError;
use windows::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
};
use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
use windows::Win32::System::Ole::CF_UNICODETEXT;

use crate::main_thread::message_window;

/// Another program may hold the clipboard open for a moment: tries this many times.
const OPEN_TRIES: u32 = 5;
const OPEN_RETRY: Duration = Duration::from_millis(10);

/// Puts `text` on the clipboard as Unicode text, its UTF-16 exactly as it is (a path with a name
/// that isn't valid Unicode keeps it). Call it on the main thread (`run_on_main`), after
/// [`main_thread::init`](crate::main_thread::init): Jerry's own window owns the clipboard.
pub fn set_text(text: &OsStr) -> Result<(), JerryError> {
    let Some(owner) = message_window() else {
        tracing::error!("clipboard used before main_thread::init");
        return Err(JerryError::internal());
    };
    let units: Vec<u16> = text.encode_wide().chain([0]).collect();
    let memory = global_copy(&units)?;
    let mut opened = Err(JerryError::internal());
    for _ in 0..OPEN_TRIES {
        // SAFETY: Jerry's own window, alive for the life of the process.
        opened = unsafe { OpenClipboard(Some(owner)) }.map_err(JerryError::from);
        if opened.is_ok() {
            break;
        }
        std::thread::sleep(OPEN_RETRY);
    }
    if let Err(error) = opened {
        free(memory);
        return Err(error);
    }
    // SAFETY: the clipboard is open for this thread, with Jerry's window as its owner.
    let set = unsafe { EmptyClipboard() }
        // SAFETY: `memory` holds NUL-terminated UTF-16; on success the clipboard owns it.
        .and_then(|()| unsafe { SetClipboardData(u32::from(CF_UNICODETEXT.0), Some(HANDLE(memory.0))) });
    if set.is_err() {
        free(memory);
    }
    // SAFETY: closes the clipboard this thread opened above.
    if let Err(error) = unsafe { CloseClipboard() } {
        tracing::warn!(%error, "couldn't close the clipboard");
    }
    set.map(|_| ()).map_err(JerryError::from)
}

/// `units` in movable global memory, as `SetClipboardData` takes it.
fn global_copy(units: &[u16]) -> Result<HGLOBAL, JerryError> {
    let bytes = size_of_val(units);
    // SAFETY: a plain allocation; freed below on failure, else owned by the clipboard.
    let memory = unsafe { GlobalAlloc(GMEM_MOVEABLE, bytes) }?;
    // SAFETY: `memory` was just allocated and is locked once, here.
    let target = unsafe { GlobalLock(memory) }.cast::<u16>();
    if target.is_null() {
        free(memory);
        return Err(JerryError::internal());
    }
    // SAFETY: `target` points to `bytes` writable bytes, which `units` fills exactly; the two
    // don't overlap.
    unsafe { std::ptr::copy_nonoverlapping(units.as_ptr(), target, units.len()) };
    // SAFETY: unlocks the lock taken above. It "fails" with no error once the count is 0.
    let _ = unsafe { GlobalUnlock(memory) };
    Ok(memory)
}

fn free(memory: HGLOBAL) {
    // SAFETY: `memory` is ours: the clipboard didn't take it.
    if let Err(error) = unsafe { GlobalFree(Some(memory)) } {
        tracing::warn!(%error, "couldn't free clipboard memory");
    }
}
