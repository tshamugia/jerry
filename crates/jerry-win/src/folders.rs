//! Windows' known folders, from the Shell rather than environment variables.

use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::path::PathBuf;

use jerry_core::JerryError;
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::UI::Shell::{
    FOLDERID_Desktop, FOLDERID_Documents, FOLDERID_Downloads, FOLDERID_LocalAppData,
    FOLDERID_Music, FOLDERID_Pictures, FOLDERID_Profile, FOLDERID_ProgramFiles,
    FOLDERID_ProgramFilesX86, FOLDERID_RoamingAppData, FOLDERID_SkyDrive, FOLDERID_UserProfiles,
    FOLDERID_Videos, FOLDERID_Windows, KF_FLAG_DEFAULT, SHGetKnownFolderPath,
};
use windows::core::GUID;

/// `%LOCALAPPDATA%`: the user's local application data (Jerry's logs live in `Jerry\logs`).
pub fn local_app_data() -> Result<PathBuf, JerryError> {
    known_folder(&FOLDERID_LocalAppData)
}

/// The folders that deleting or moving needs an extra confirmation for (data safety rule 6):
/// Windows, Program Files and Program Files (x86), and the Users root. One the Shell doesn't
/// know is left out. Check paths against them with `jerry_core::delete::takes_system_folder`.
pub fn system_folders() -> Vec<PathBuf> {
    [
        &FOLDERID_Windows,
        &FOLDERID_ProgramFiles,
        &FOLDERID_ProgramFilesX86,
        &FOLDERID_UserProfiles,
    ]
    .into_iter()
    .filter_map(|id| known_folder(id).ok())
    .collect()
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

/// The short names the address bar takes for known folders (`Downloads`), as it suggests them.
pub const SHORT_NAMES: &[&str] = &[
    "Desktop",
    "Documents",
    "Downloads",
    "Music",
    "OneDrive",
    "Pictures",
    "Videos",
];

/// The known folder a short name stands for, case-insensitive, also as Explorer writes it
/// (`shell:Downloads`): one of [`SHORT_NAMES`], or `Home` for the user's own folder. `None` for
/// another name, or a folder this user doesn't have.
pub fn by_short_name(name: &str) -> Option<PathBuf> {
    let lower = name.to_ascii_lowercase();
    let id = match lower.strip_prefix("shell:").unwrap_or(&lower) {
        "desktop" => FOLDERID_Desktop,
        "documents" | "personal" => FOLDERID_Documents,
        "downloads" => FOLDERID_Downloads,
        "music" => FOLDERID_Music,
        "onedrive" => FOLDERID_SkyDrive,
        "pictures" => FOLDERID_Pictures,
        "videos" => FOLDERID_Videos,
        "home" | "profile" => FOLDERID_Profile,
        _ => return None,
    };
    known_folder(&id).ok()
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

    #[test]
    fn short_names_lead_to_known_folders() {
        let downloads = by_short_name("Downloads").expect("the user has Downloads");
        assert!(downloads.is_absolute(), "{}", downloads.display());
        assert_eq!(by_short_name("shell:downloads"), Some(downloads));
        assert_eq!(by_short_name("HOME"), profile().ok());
        assert_eq!(by_short_name("Downloads2"), None);
        for name in SHORT_NAMES {
            // OneDrive may not be set up on a build machine; the others always are.
            if *name != "OneDrive" {
                assert!(by_short_name(name).is_some(), "{name}");
            }
        }
    }

    #[test]
    fn system_folders_are_windows_program_files_and_users() {
        let folders = system_folders();
        assert_eq!(folders.len(), 4, "{folders:?}");
        let windows = std::env::var_os("SystemRoot").expect("SystemRoot");
        assert!(
            folders
                .iter()
                .any(|folder| folder.as_os_str().eq_ignore_ascii_case(&windows)),
            "{folders:?}"
        );
        let profile = std::env::var_os("USERPROFILE").expect("USERPROFILE");
        let users = PathBuf::from(profile)
            .parent()
            .expect("the Users root")
            .to_path_buf();
        assert!(
            folders
                .iter()
                .any(|folder| folder.as_os_str().eq_ignore_ascii_case(users.as_os_str())),
            "{folders:?}"
        );
    }
}
