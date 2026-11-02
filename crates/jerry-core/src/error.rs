//! The one error shape Jerry returns: a code with its parameters, and the path it happened on.
//! Never text: the UI turns the code into English in one place (`src/strings/errors.ts`).
//!
//! On the wire a [`JerryError`] is `{ code, params, path }`, for example
//! `{ "code": "InUse", "params": { "app": "EXCEL.EXE" }, "path": "C:\\Work\\plan.dwg" }`. Codes
//! without parameters have no `params`.

use std::io;
use std::path::{Path, PathBuf};

use serde::{Serialize, Serializer};
use specta::Type;
use windows::Win32::Foundation::{
    E_ABORT, ERROR_ACCESS_DENIED, ERROR_BAD_NET_NAME, ERROR_BAD_NETPATH, ERROR_BAD_PATHNAME,
    ERROR_CANCELLED, ERROR_DEV_NOT_EXIST, ERROR_DEVICE_NOT_CONNECTED, ERROR_DEVICE_REMOVED,
    ERROR_DIRECTORY, ERROR_DISK_FULL, ERROR_DISK_QUOTA_EXCEEDED, ERROR_FILE_NOT_FOUND,
    ERROR_FILENAME_EXCED_RANGE, ERROR_HANDLE_DISK_FULL, ERROR_INVALID_DRIVE, ERROR_INVALID_NAME,
    ERROR_LOCK_VIOLATION, ERROR_NETNAME_DELETED, ERROR_NETWORK_ACCESS_DENIED,
    ERROR_NO_MEDIA_IN_DRIVE, ERROR_NOT_READY, ERROR_OPERATION_ABORTED, ERROR_PATH_NOT_FOUND,
    ERROR_PRIVILEGE_NOT_HELD, ERROR_REQUEST_ABORTED, ERROR_SHARING_VIOLATION, ERROR_UNEXP_NET_ERR,
    ERROR_USER_MAPPED_FILE, ERROR_WRITE_PROTECT, WIN32_ERROR,
};
use windows::core::HRESULT;

/// What went wrong, with the parameters the UI needs to explain it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(tag = "code", content = "params")]
pub enum ErrorCode {
    /// Windows refused access: permissions, or a write-protected device.
    AccessDenied,
    /// The file, folder or share isn't there.
    NotFound,
    /// Another program has the file open.
    InUse {
        /// The program holding the file, when Windows knows it.
        app: Option<String>,
    },
    /// The destination is full.
    NoSpace {
        /// Bytes the operation needs, when it was checked up front.
        #[specta(type = Option<f64>)]
        needed: Option<u64>,
        /// Bytes free at the destination, when it was checked up front.
        #[specta(type = Option<f64>)]
        available: Option<u64>,
    },
    /// The name isn't allowed here.
    InvalidName,
    /// The drive or network location went away: unplugged, disconnected, no media.
    DeviceGone,
    /// The location has no Recycle Bin, so deleting there would be permanent.
    NoRecycleBin,
    /// The user cancelled.
    Cancelled,
    /// Any other Windows error.
    Os {
        /// The Win32 error code, or the `HRESULT` (as `u32`) when Windows reported one.
        os: u32,
    },
    /// Jerry's own bug, for example a job that panicked. The details are in the log.
    Internal,
}

impl ErrorCode {
    /// The code for a Win32 error (`GetLastError`).
    pub fn from_win32(code: u32) -> Self {
        match WIN32_ERROR(code) {
            ERROR_ACCESS_DENIED
            | ERROR_NETWORK_ACCESS_DENIED
            | ERROR_PRIVILEGE_NOT_HELD
            | ERROR_WRITE_PROTECT => Self::AccessDenied,
            ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND | ERROR_BAD_NET_NAME | ERROR_DIRECTORY => {
                Self::NotFound
            }
            ERROR_SHARING_VIOLATION | ERROR_LOCK_VIOLATION | ERROR_USER_MAPPED_FILE => {
                Self::InUse { app: None }
            }
            ERROR_DISK_FULL | ERROR_HANDLE_DISK_FULL | ERROR_DISK_QUOTA_EXCEEDED => Self::NoSpace {
                needed: None,
                available: None,
            },
            ERROR_INVALID_NAME | ERROR_BAD_PATHNAME | ERROR_FILENAME_EXCED_RANGE => {
                Self::InvalidName
            }
            ERROR_NOT_READY
            | ERROR_INVALID_DRIVE
            | ERROR_NO_MEDIA_IN_DRIVE
            | ERROR_DEV_NOT_EXIST
            | ERROR_DEVICE_NOT_CONNECTED
            | ERROR_DEVICE_REMOVED
            | ERROR_BAD_NETPATH
            | ERROR_NETNAME_DELETED
            | ERROR_UNEXP_NET_ERR => Self::DeviceGone,
            ERROR_CANCELLED | ERROR_REQUEST_ABORTED | ERROR_OPERATION_ABORTED => Self::Cancelled,
            _ => Self::Os { os: code },
        }
    }

    /// The code for an `HRESULT`. Win32 errors wrapped in one (`HRESULT_FROM_WIN32`) map like
    /// [`ErrorCode::from_win32`].
    pub fn from_hresult(hr: HRESULT) -> Self {
        const FACILITY_WIN32_FAILURE: u32 = 0x8007_0000;
        let bits = hr.0 as u32;
        if bits & 0xFFFF_0000 == FACILITY_WIN32_FAILURE {
            Self::from_win32(bits & 0xFFFF)
        } else if hr == E_ABORT {
            Self::Cancelled
        } else {
            Self::Os { os: bits }
        }
    }
}

/// An error from a core or Windows operation.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, Serialize, Type)]
#[error("{code:?}{}", path_suffix(.path))]
pub struct JerryError {
    /// The code and its parameters (`code` and `params` on the wire).
    #[serde(flatten)]
    pub code: ErrorCode,
    /// The file or folder it happened on. Exact here; the UI gets display text only.
    #[serde(serialize_with = "serialize_display_path")]
    #[specta(type = Option<String>)]
    pub path: Option<PathBuf>,
}

impl JerryError {
    /// An error with no path.
    pub fn new(code: ErrorCode) -> Self {
        Self { code, path: None }
    }

    /// The same error, on `path`.
    #[must_use]
    pub fn at(mut self, path: impl Into<PathBuf>) -> Self {
        self.path = Some(path.into());
        self
    }

    /// An error from a Win32 error code (`GetLastError`).
    pub fn from_win32(code: u32) -> Self {
        Self::new(ErrorCode::from_win32(code))
    }

    /// Jerry's own bug (see [`ErrorCode::Internal`]). Log the details where it happens.
    pub fn internal() -> Self {
        Self::new(ErrorCode::Internal)
    }
}

impl From<io::Error> for JerryError {
    fn from(error: io::Error) -> Self {
        if let Some(code) = error.raw_os_error() {
            return Self::from_win32(code as u32);
        }
        // No Windows code: std made the error itself.
        let code = match error.kind() {
            io::ErrorKind::NotFound => ErrorCode::NotFound,
            io::ErrorKind::PermissionDenied | io::ErrorKind::ReadOnlyFilesystem => {
                ErrorCode::AccessDenied
            }
            io::ErrorKind::InvalidFilename => ErrorCode::InvalidName,
            io::ErrorKind::StorageFull | io::ErrorKind::QuotaExceeded => ErrorCode::NoSpace {
                needed: None,
                available: None,
            },
            io::ErrorKind::ResourceBusy | io::ErrorKind::ExecutableFileBusy => {
                ErrorCode::InUse { app: None }
            }
            _ => {
                tracing::warn!(%error, "I/O error without a Windows code");
                ErrorCode::Internal
            }
        };
        Self::new(code)
    }
}

impl From<windows::core::Error> for JerryError {
    fn from(error: windows::core::Error) -> Self {
        Self::new(ErrorCode::from_hresult(error.code()))
    }
}

/// `path` as the user knows it: without the `\\?\` prefix core uses for long paths.
pub fn display_path(path: &Path) -> String {
    let text = path.to_string_lossy();
    if let Some(share) = text.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{share}")
    } else if let Some(local) = text.strip_prefix(r"\\?\") {
        local.to_owned()
    } else {
        text.into_owned()
    }
}

fn path_suffix(path: &Option<PathBuf>) -> String {
    path.as_deref()
        .map(|path| format!(" ({})", display_path(path)))
        .unwrap_or_default()
}

fn serialize_display_path<S: Serializer>(
    path: &Option<PathBuf>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    path.as_deref().map(display_path).serialize(serializer)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn win32_codes_map_to_jerry_codes() {
        let cases = [
            (5, ErrorCode::AccessDenied),           // ERROR_ACCESS_DENIED
            (19, ErrorCode::AccessDenied),          // ERROR_WRITE_PROTECT
            (1314, ErrorCode::AccessDenied),        // ERROR_PRIVILEGE_NOT_HELD
            (2, ErrorCode::NotFound),               // ERROR_FILE_NOT_FOUND
            (3, ErrorCode::NotFound),               // ERROR_PATH_NOT_FOUND
            (67, ErrorCode::NotFound),              // ERROR_BAD_NET_NAME
            (32, ErrorCode::InUse { app: None }),   // ERROR_SHARING_VIOLATION
            (33, ErrorCode::InUse { app: None }),   // ERROR_LOCK_VIOLATION
            (1224, ErrorCode::InUse { app: None }), // ERROR_USER_MAPPED_FILE
            (
                112, // ERROR_DISK_FULL
                ErrorCode::NoSpace {
                    needed: None,
                    available: None,
                },
            ),
            (123, ErrorCode::InvalidName),  // ERROR_INVALID_NAME
            (206, ErrorCode::InvalidName),  // ERROR_FILENAME_EXCED_RANGE
            (21, ErrorCode::DeviceGone),    // ERROR_NOT_READY
            (53, ErrorCode::DeviceGone),    // ERROR_BAD_NETPATH
            (64, ErrorCode::DeviceGone),    // ERROR_NETNAME_DELETED
            (1167, ErrorCode::DeviceGone),  // ERROR_DEVICE_NOT_CONNECTED
            (1223, ErrorCode::Cancelled),   // ERROR_CANCELLED
            (1235, ErrorCode::Cancelled),   // ERROR_REQUEST_ABORTED
            (87, ErrorCode::Os { os: 87 }), // ERROR_INVALID_PARAMETER
        ];
        for (win32, code) in cases {
            assert_eq!(ErrorCode::from_win32(win32), code, "Win32 error {win32}");
        }
    }

    #[test]
    fn hresults_unwrap_win32_codes() {
        let wrapped = |code: u32| HRESULT((0x8007_0000 | code) as i32);
        assert_eq!(ErrorCode::from_hresult(wrapped(5)), ErrorCode::AccessDenied);
        assert_eq!(ErrorCode::from_hresult(wrapped(1223)), ErrorCode::Cancelled);
        assert_eq!(ErrorCode::from_hresult(E_ABORT), ErrorCode::Cancelled);
        // COPYENGINE_E_RECYCLE_BIN_NOT_FOUND: not a Win32 code, kept as is for jerry-win.
        let copy_engine = HRESULT(0x8027_0064_u32 as i32);
        assert_eq!(
            ErrorCode::from_hresult(copy_engine),
            ErrorCode::Os { os: 0x8027_0064 }
        );
        let error = windows::core::Error::from_hresult(wrapped(32));
        assert_eq!(JerryError::from(error).code, ErrorCode::InUse { app: None });
    }

    #[test]
    fn io_errors_map_by_os_code_then_kind() {
        let sharing = io::Error::from_raw_os_error(32);
        assert_eq!(
            JerryError::from(sharing).code,
            ErrorCode::InUse { app: None }
        );
        let not_found = io::Error::from(io::ErrorKind::NotFound);
        assert_eq!(JerryError::from(not_found).code, ErrorCode::NotFound);
        let eof = io::Error::from(io::ErrorKind::UnexpectedEof);
        assert_eq!(JerryError::from(eof).code, ErrorCode::Internal);
    }

    #[test]
    fn a_real_windows_error_keeps_its_path() {
        let missing = std::env::temp_dir()
            .join(format!("jerry-missing-{}", std::process::id()))
            .join("ნახაზი.dwg");
        let error = std::fs::metadata(&missing)
            .map(|_| ())
            .map_err(|error| JerryError::from(error).at(&missing))
            .expect_err("the folder doesn't exist");
        assert_eq!(error.code, ErrorCode::NotFound);
        assert_eq!(error.path.as_deref(), Some(missing.as_path()));
    }

    #[test]
    fn serializes_to_code_params_and_display_path() {
        let in_use = JerryError::new(ErrorCode::InUse {
            app: Some("EXCEL.EXE".into()),
        })
        .at(r"\\?\C:\Work\ნახაზი.dwg");
        assert_eq!(
            serde_json::to_value(&in_use).expect("serializes"),
            serde_json::json!({
                "code": "InUse",
                "params": { "app": "EXCEL.EXE" },
                "path": r"C:\Work\ნახაზი.dwg",
            })
        );

        let no_space = JerryError::new(ErrorCode::NoSpace {
            needed: Some(5_000_000_000),
            available: None,
        });
        assert_eq!(
            serde_json::to_value(&no_space).expect("serializes"),
            serde_json::json!({
                "code": "NoSpace",
                "params": { "needed": 5_000_000_000_u64, "available": null },
                "path": null,
            })
        );

        let not_found = JerryError::new(ErrorCode::NotFound);
        assert_eq!(
            serde_json::to_value(&not_found).expect("serializes"),
            serde_json::json!({ "code": "NotFound", "path": null })
        );
    }

    #[test]
    fn display_path_drops_the_long_path_prefix() {
        assert_eq!(display_path(Path::new(r"\\?\C:\a\b")), r"C:\a\b");
        assert_eq!(
            display_path(Path::new(r"\\?\UNC\server\share\a")),
            r"\\server\share\a"
        );
        assert_eq!(display_path(Path::new(r"D:\ქართული")), r"D:\ქართული");
    }

    #[test]
    fn display_names_the_code_and_path() {
        let error = JerryError::from_win32(5).at(r"\\?\C:\Windows\x");
        assert_eq!(error.to_string(), r"AccessDenied (C:\Windows\x)");
        assert_eq!(JerryError::internal().to_string(), "Internal");
    }
}
