//! The one error shape the core returns: a code, the path it happened on, and nothing else. The
//! UI turns the code into English text. Spike-sized: the full code list arrives with its own issue.

use std::path::{Path, PathBuf};

/// What went wrong.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    AccessDenied,
    NotFound,
    Cancelled,
    /// Any other Windows error, with its Win32 code.
    Os(u32),
}

/// An error from a core operation.
#[derive(Debug, Clone, thiserror::Error)]
#[error("{code:?} ({})", path.as_deref().map(Path::display).map(|p| p.to_string()).unwrap_or_default())]
pub struct JerryError {
    pub code: ErrorCode,
    pub path: Option<PathBuf>,
}

impl JerryError {
    pub fn new(code: ErrorCode, path: &Path) -> Self {
        Self {
            code,
            path: Some(path.to_path_buf()),
        }
    }

    /// Maps a Win32 error code (`GetLastError`) to an error code.
    pub fn from_win32(code: u32, path: &Path) -> Self {
        const ERROR_FILE_NOT_FOUND: u32 = 2;
        const ERROR_PATH_NOT_FOUND: u32 = 3;
        const ERROR_ACCESS_DENIED: u32 = 5;
        const ERROR_CANCELLED: u32 = 1223;
        let code = match code {
            ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND => ErrorCode::NotFound,
            ERROR_ACCESS_DENIED => ErrorCode::AccessDenied,
            ERROR_CANCELLED => ErrorCode::Cancelled,
            other => ErrorCode::Os(other),
        };
        Self::new(code, path)
    }

    pub fn from_io(error: &std::io::Error, path: &Path) -> Self {
        match error.raw_os_error() {
            Some(code) => Self::from_win32(code as u32, path),
            None if error.kind() == std::io::ErrorKind::NotFound => {
                Self::new(ErrorCode::NotFound, path)
            }
            None => Self::new(ErrorCode::Os(0), path),
        }
    }
}
