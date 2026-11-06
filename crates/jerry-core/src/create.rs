//! Making new items in a folder (PRD «შექმნა», #36): a new folder, named as Explorer names them.
//!
//! Blocking IO: run it on a background thread.

use std::io;
use std::path::Path;

use crate::error::{ErrorCode, JerryError};
use crate::name::check_name;

/// How far the number after a taken name goes: `New folder (9999)`.
const MAX_NUMBER: u32 = 9_999;

/// Makes a folder named `name` in `folder` (a long path, as core keeps them), or `name (2)`,
/// `name (3)`… when an item there has the name, as Explorer does. Returns the name it got.
/// Fails with [`ErrorCode::InvalidName`] for a name that breaks a naming rule, and with the
/// folder's path for anything Windows refuses there ([`ErrorCode::AccessDenied`],
/// [`ErrorCode::NotFound`] for a folder that is gone).
pub fn create_folder(folder: &Path, name: &str) -> Result<String, JerryError> {
    check_name(name).map_err(|_| JerryError::new(ErrorCode::InvalidName))?;
    for number in 1..=MAX_NUMBER {
        let candidate = if number == 1 {
            name.to_owned()
        } else {
            format!("{name} ({number})")
        };
        // The number can make a long name too long.
        check_name(&candidate).map_err(|_| JerryError::new(ErrorCode::InvalidName))?;
        match std::fs::create_dir(folder.join(&candidate)) {
            Ok(()) => return Ok(candidate),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(JerryError::from(error).at(folder)),
        }
    }
    Err(JerryError::new(ErrorCode::InvalidName).at(folder))
}
