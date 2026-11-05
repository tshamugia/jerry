//! Paths as core uses them: absolute and with the `\\?\` prefix, so Win32 calls take paths over
//! 260 characters. Core keeps them exact (`OsString`); the UI gets display text only
//! ([`display_path`](crate::error::display_path)).

use std::ffi::OsString;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};

use crate::error::{ErrorCode, JerryError};

/// `path` with the long-path prefix: `\\?\C:\…` for drives, `\\?\UNC\server\share\…` for shares.
/// A path that already has a `\\?\` or `\\.\` prefix stays as it is.
///
/// The prefix turns off Windows' own path parsing, so this first makes the path absolute and
/// normalizes it (`GetFullPathNameW`: `/` to `\`, `.` and `..` resolved). An empty path is
/// [`ErrorCode::InvalidName`].
pub fn long_path(path: &Path) -> Result<PathBuf, JerryError> {
    let absolute =
        std::path::absolute(path).map_err(|_| JerryError::new(ErrorCode::InvalidName).at(path))?;
    let wide: Vec<u16> = absolute.as_os_str().encode_wide().collect();
    let starts_with = |prefix: &str| wide.starts_with(&prefix.encode_utf16().collect::<Vec<_>>());
    if starts_with(r"\\?\") || starts_with(r"\\.\") {
        Ok(absolute)
    } else if starts_with(r"\\") {
        let mut long = OsString::from(r"\\?\UNC\");
        long.push(OsString::from_wide(&wide[2..]));
        Ok(PathBuf::from(long))
    } else {
        let mut long = OsString::from(r"\\?\");
        long.push(absolute.as_os_str());
        Ok(PathBuf::from(long))
    }
}

/// `path` without the long-path prefix, exactly: `C:\…` for `\\?\C:\…`, `\\server\share\…` for
/// `\\?\UNC\server\share\…`. For programs that don't take the prefix (the Shell's open, the
/// clipboard's text); names that aren't valid Unicode stay as they are. Other paths, a device
/// path (`\\.\`) among them, come back unchanged.
pub fn plain_path(path: &Path) -> PathBuf {
    let wide: Vec<u16> = path.as_os_str().encode_wide().collect();
    let strip = |prefix: &str| {
        let prefix: Vec<u16> = prefix.encode_utf16().collect();
        wide.starts_with(&prefix).then(|| &wide[prefix.len()..])
    };
    if let Some(share) = strip(r"\\?\UNC\") {
        let mut plain: Vec<u16> = r"\\".encode_utf16().collect();
        plain.extend_from_slice(share);
        PathBuf::from(OsString::from_wide(&plain))
    } else if let Some(local) = strip(r"\\?\") {
        PathBuf::from(OsString::from_wide(local))
    } else {
        path.to_path_buf()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_paths_lose_only_the_prefix() {
        let plain = |path: &str| plain_path(Path::new(path));
        assert_eq!(
            plain(r"\\?\C:\Work\ნახაზები"),
            PathBuf::from(r"C:\Work\ნახაზები")
        );
        assert_eq!(plain(r"\\?\C:\"), PathBuf::from(r"C:\"));
        assert_eq!(
            plain(r"\\?\UNC\server\share\a"),
            PathBuf::from(r"\\server\share\a")
        );
        assert_eq!(plain(r"C:\Work"), PathBuf::from(r"C:\Work"));
        assert_eq!(
            plain(r"\\.\PhysicalDrive0"),
            PathBuf::from(r"\\.\PhysicalDrive0")
        );
    }

    #[test]
    fn plain_paths_keep_names_that_are_not_unicode() {
        // An unpaired surrogate, as NTFS allows in a name.
        let mut wide: Vec<u16> = r"\\?\C:\a".encode_utf16().collect();
        wide.push(0xD800);
        let plain = plain_path(&PathBuf::from(OsString::from_wide(&wide)));
        let mut expected: Vec<u16> = r"C:\a".encode_utf16().collect();
        expected.push(0xD800);
        assert_eq!(
            plain.as_os_str().encode_wide().collect::<Vec<_>>(),
            expected
        );
    }

    #[test]
    fn drives_and_shares_get_the_prefix() {
        let long = |path: &str| long_path(Path::new(path)).expect("absolute path");
        assert_eq!(long(r"C:\Work"), PathBuf::from(r"\\?\C:\Work"));
        assert_eq!(long(r"C:\"), PathBuf::from(r"\\?\C:\"));
        assert_eq!(
            long(r"\\server\share\ნახაზები"),
            PathBuf::from(r"\\?\UNC\server\share\ნახაზები")
        );
    }

    #[test]
    fn normalizes_before_adding_the_prefix() {
        let long = |path: &str| long_path(Path::new(path)).expect("absolute path");
        assert_eq!(long("C:/Work/a/../b"), PathBuf::from(r"\\?\C:\Work\b"));
        assert_eq!(long(r"C:\Work\.\b"), PathBuf::from(r"\\?\C:\Work\b"));
    }

    #[test]
    fn prefixed_paths_stay_as_they_are() {
        let long = |path: &str| long_path(Path::new(path)).expect("absolute path");
        assert_eq!(long(r"\\?\C:\a\..\b"), PathBuf::from(r"\\?\C:\a\..\b"));
        assert_eq!(
            long(r"\\?\UNC\server\share"),
            PathBuf::from(r"\\?\UNC\server\share")
        );
        assert_eq!(
            long(r"\\.\PhysicalDrive0"),
            PathBuf::from(r"\\.\PhysicalDrive0")
        );
    }

    #[test]
    fn an_empty_path_is_an_invalid_name() {
        let error = long_path(Path::new("")).expect_err("no path");
        assert_eq!(error.code, ErrorCode::InvalidName);
    }
}
