//! Windows' known folders, from the Shell rather than environment variables.

use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::path::PathBuf;

use jerry_core::JerryError;
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::UI::Shell::{
    FOLDERID_LocalAppData, FOLDERID_Profile, FOLDERID_RoamingAppData, KF_FLAG_DEFAULT,
    SHGetKnownFolderPath,
};
use windows::core::GUID;

/// `%LOCALAPPDATA%`: the user's local application data (Jerry's logs live in `Jerry\logs`).
pub fn local_app_data() -> Result<PathBuf, JerryError> {
    known_folder(&FOLDERID_LocalAppData)
}

/// `%APPDATA%`: the user's roaming application data (Jerry's database is `Jerry\jerry.db`).
pub fn roaming_app_data() -> Result<PathBuf, JerryError> {
    known_folder(&FOLDERID_RoamingAppData)
}

/// The user's own folder (`C:\Users\name`): where a tab opens when there is nothing else to
/// open.
pub fn profile() -> Result<PathBuf, JerryError> {
    known_folder(&FOLDERID_Profile)
}

pub(crate) fn known_folder(id: &GUID) -> Result<PathBuf, JerryError> {
    // SAFETY: `id` is a valid known folder id; no access token means the current user.
    let raw = unsafe { SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None) }?;
    // SAFETY: on success the Shell returns a valid NUL-terminated string.
    let path = PathBuf::from(OsString::from_wide(unsafe { raw.as_wide() }));
    // SAFETY: the string was allocated with the COM allocator and is freed once, here.
    unsafe { CoTaskMemFree(Some(raw.0.cast_const().cast())) };
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_data_folders_exist() {
        for path in [local_app_data(), roaming_app_data()] {
            let path = path.expect("the Shell knows the folder");
            assert!(path.is_absolute(), "{}", path.display());
            assert!(path.is_dir(), "{}", path.display());
        }
        assert_ne!(local_app_data(), roaming_app_data());
        let profile = profile().expect("the Shell knows the profile");
        assert!(profile.is_dir(), "{}", profile.display());
        assert!(
            local_app_data().expect("local").starts_with(&profile),
            "AppData is in the profile"
        );
    }
}
