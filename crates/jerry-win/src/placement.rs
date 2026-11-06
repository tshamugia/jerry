//! A window's place on the screen, for the session (#42): its restored rectangle and whether it
//! is maximized, read with `GetWindowPlacement` and put back with `SetWindowPlacement` while the
//! window is still hidden. Both work in Windows' workspace coordinates, so a placement read here
//! goes back exactly; a rectangle that would be off every screen (a monitor that is gone),
//! Windows moves onto one.

use jerry_core::JerryError;
use jerry_core::session::Placement;
use windows::Win32::Foundation::{HWND, POINT, RECT};
use windows::Win32::UI::WindowsAndMessaging::{
    GetWindowPlacement, SW_HIDE, SW_SHOWMAXIMIZED, SW_SHOWMINIMIZED, SetWindowPlacement,
    WINDOWPLACEMENT, WINDOWPLACEMENT_FLAGS, WPF_RESTORETOMAXIMIZED,
};

/// Placements smaller than this are a damaged session, not a window someone sized.
const SMALLEST: i32 = 100;

/// Where window `hwnd` is: its restored rectangle (also while it is maximized or minimized) and
/// whether it is maximized, or minimized from maximized.
pub fn get(hwnd: isize) -> Result<Placement, JerryError> {
    let mut placement = WINDOWPLACEMENT {
        length: size_of::<WINDOWPLACEMENT>() as u32,
        ..Default::default()
    };
    // SAFETY: `placement` is a writable WINDOWPLACEMENT with its length set; a stale handle only
    // makes the call fail.
    unsafe { GetWindowPlacement(HWND(hwnd as _), &mut placement) }?;
    let shown = placement.showCmd as i32;
    let maximized = shown == SW_SHOWMAXIMIZED.0
        || (shown == SW_SHOWMINIMIZED.0 && placement.flags.contains(WPF_RESTORETOMAXIMIZED));
    let rect = placement.rcNormalPosition;
    Ok(Placement {
        left: rect.left,
        top: rect.top,
        right: rect.right,
        bottom: rect.bottom,
        maximized,
    })
}

/// Moves hidden window `hwnd` to `placement`'s restored rectangle and leaves it hidden; showing it
/// maximized is the caller's (Tauri's `maximize` before `show`). Returns `false`, changing
/// nothing, for a rectangle too small to be a real window. Call on the window's thread.
pub fn set_hidden(hwnd: isize, placement: &Placement) -> Result<bool, JerryError> {
    let (width, height) = (
        placement.right.saturating_sub(placement.left),
        placement.bottom.saturating_sub(placement.top),
    );
    if width < SMALLEST || height < SMALLEST {
        return Ok(false);
    }
    let unset = POINT { x: -1, y: -1 };
    let wanted = WINDOWPLACEMENT {
        length: size_of::<WINDOWPLACEMENT>() as u32,
        flags: WINDOWPLACEMENT_FLAGS(0),
        showCmd: SW_HIDE.0 as u32,
        ptMinPosition: unset,
        ptMaxPosition: unset,
        rcNormalPosition: RECT {
            left: placement.left,
            top: placement.top,
            right: placement.right,
            bottom: placement.bottom,
        },
    };
    // SAFETY: `wanted` is a complete WINDOWPLACEMENT with its length set; a stale handle only
    // makes the call fail.
    unsafe { SetWindowPlacement(HWND(hwnd as _), &wanted) }?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiny_placements_are_left_alone() {
        let tiny = Placement {
            left: 10,
            top: 10,
            right: 20,
            bottom: 600,
            maximized: false,
        };
        // Checked before Windows is asked, so no window is needed.
        assert_eq!(set_hidden(0, &tiny), Ok(false));
        assert!(get(0).is_err(), "no such window");
    }
}
