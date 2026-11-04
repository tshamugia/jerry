//! The parts of the Windows look Jerry follows: the app theme, the accent color and the Mica
//! backdrop.

use std::ffi::c_void;

use jerry_core::JerryError;
use windows::Foundation::TypedEventHandler;
use windows::UI::Color;
use windows::UI::ViewManagement::{UIColorType, UISettings};
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Dwm::{
    DWM_SYSTEMBACKDROP_TYPE, DWMSBT_MAINWINDOW, DWMSBT_NONE, DWMWA_SYSTEMBACKDROP_TYPE,
    DwmSetWindowAttribute,
};
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
use windows::core::{IInspectable, Ref, w};

use crate::os_build;

/// First build with the documented `DWMWA_SYSTEMBACKDROP_TYPE` (Windows 11 22H2). 21H2 had only
/// an undocumented attribute and is out of support, so it gets the flat color like Windows 10.
pub const BACKDROP_BUILD: u32 = 22_621;

/// Whether Windows' app mode is dark ("Choose your app mode" in Settings → Personalization →
/// Colors). Read from HKCU, so a window can get its theme background before it exists.
pub fn apps_use_dark_theme() -> bool {
    let mut value = 1u32;
    let mut size = size_of::<u32>() as u32;
    // SAFETY: `value` is a writable u32 and `size` holds its size, as RRF_RT_REG_DWORD needs.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"),
            w!("AppsUseLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some((&raw mut value).cast()),
            Some(&mut size),
        )
    };
    status.is_ok() && value == 0
}

/// Whether this Windows can draw Mica.
pub fn mica_supported() -> bool {
    os_build().is_some_and(|build| build >= BACKDROP_BUILD)
}

/// Turns Mica on or off behind the window `hwnd` (raw `HWND`). Mica shows only through
/// transparent pixels: the window and its WebView2 must be transparent and the UI must leave its
/// Mica areas unpainted. Fails on a Windows without Mica ([`mica_supported`]).
pub fn set_mica(hwnd: isize, enabled: bool) -> Result<(), JerryError> {
    if !mica_supported() {
        return Err(JerryError::internal());
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
    Ok(())
}

/// The Windows accent color as Fluent uses it, each `0xRRGGBB`: `light` on light surfaces
/// (AccentDark1), `dark` on dark ones (AccentLight2). For the default blue (#0078D4) these are
/// #005FB8 and #60CDFF, the design's accent colors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AccentColors {
    pub light: u32,
    pub dark: u32,
}

/// The current Windows accent.
pub fn accent_colors() -> Result<AccentColors, JerryError> {
    read_accent(&UISettings::new()?)
}

/// Follows the Windows accent: calls its handler with the accent each time Windows' colors
/// change (the accent, or the app theme), on a thread of Windows' choosing, until dropped.
pub struct AccentWatch {
    settings: UISettings,
    token: i64,
}

impl AccentWatch {
    pub fn start(on_change: impl Fn(AccentColors) + Send + 'static) -> Result<Self, JerryError> {
        let settings = UISettings::new()?;
        let handler =
            TypedEventHandler::new(move |sender: Ref<UISettings>, _: Ref<IInspectable>| {
                if let Some(settings) = sender.as_ref() {
                    match read_accent(settings) {
                        Ok(colors) => on_change(colors),
                        Err(error) => tracing::warn!(%error, "can't read the Windows accent"),
                    }
                }
                Ok(())
            });
        let token = settings.ColorValuesChanged(&handler)?;
        Ok(Self { settings, token })
    }
}

impl Drop for AccentWatch {
    fn drop(&mut self) {
        if let Err(error) = self.settings.RemoveColorValuesChanged(self.token) {
            tracing::warn!(%error, "can't stop following the Windows accent");
        }
    }
}

fn read_accent(settings: &UISettings) -> Result<AccentColors, JerryError> {
    Ok(AccentColors {
        light: rgb(settings.GetColorValue(UIColorType::AccentDark1)?),
        dark: rgb(settings.GetColorValue(UIColorType::AccentLight2)?),
    })
}

fn rgb(color: Color) -> u32 {
    (u32::from(color.R) << 16) | (u32::from(color.G) << 8) | u32::from(color.B)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors_become_rrggbb() {
        let color = Color {
            A: 255,
            R: 0x00,
            G: 0x5f,
            B: 0xb8,
        };
        assert_eq!(rgb(color), 0x005fb8);
    }

    #[test]
    fn mica_needs_windows_11_22h2() {
        let expected = os_build().is_some_and(|build| build >= 22_621);
        assert_eq!(mica_supported(), expected);
        if !expected {
            assert!(set_mica(0, true).is_err());
        }
    }

    #[test]
    fn the_app_theme_reads_without_panicking() {
        // Either answer is fine; CI runners have no user preference.
        let _ = apps_use_dark_theme();
    }
}
