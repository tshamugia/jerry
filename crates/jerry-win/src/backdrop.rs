//! The window backdrop: Mica on Windows 11, a flat color elsewhere.

use std::ffi::c_void;

use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Dwm::{
    DWM_SYSTEMBACKDROP_TYPE, DWMSBT_MAINWINDOW, DWMSBT_NONE, DWMWA_SYSTEMBACKDROP_TYPE,
    DwmSetWindowAttribute,
};
use windows::core::Result;

use crate::os_build;

/// First build with the documented `DWMWA_SYSTEMBACKDROP_TYPE` (Windows 11 22H2). 21H2 had only
/// an undocumented attribute and is out of support, so it gets the flat color like Windows 10.
pub const BACKDROP_TYPE_BUILD: u32 = 22_621;

/// Whether this Windows can draw Mica.
pub fn mica_supported() -> bool {
    os_build().is_some_and(|build| build >= BACKDROP_TYPE_BUILD)
}

/// Turns Mica on or off for `hwnd` (raw `HWND`) and returns whether it is now on. Mica shows
/// only through transparent pixels, so the window and the WebView2 must be transparent and the
/// UI must leave its Mica areas unpainted.
pub fn set_mica(hwnd: isize, enabled: bool) -> Result<bool> {
    if !mica_supported() {
        return Ok(false);
    }
    let backdrop: DWM_SYSTEMBACKDROP_TYPE = if enabled {
        DWMSBT_MAINWINDOW
    } else {
        DWMSBT_NONE
    };
    // SAFETY: the attribute takes a DWM_SYSTEMBACKDROP_TYPE; we pass its address and size.
    unsafe {
        DwmSetWindowAttribute(
            HWND(hwnd as *mut c_void),
            DWMWA_SYSTEMBACKDROP_TYPE,
            (&raw const backdrop).cast(),
            size_of::<DWM_SYSTEMBACKDROP_TYPE>() as u32,
        )
    }?;
    Ok(enabled)
}
