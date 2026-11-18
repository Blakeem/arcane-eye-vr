//! A Windows desktop of its own for a process and every process it starts, so their windows take
//! neither the keyboard focus nor the mouse of the desktop the user works on.
//!
//! A process started with `STARTUPINFO.lpDesktop` set to the desktop's name runs there, and the
//! processes it starts inherit the desktop. The trial runner runs a copy of itself on such a desktop.
//! The copy and every process it starts end with the run, so no window stays on a desktop nobody
//! sees.

use std::ffi::OsString;
use std::path::PathBuf;

use crate::raw::desktop as raw_desktop;
pub use crate::raw::desktop::Desktop;

/// The character that a desktop name may not hold. It separates the window station from the desktop
/// in a full desktop name.
const STATION_SEPARATOR: char = '\\';

/// Why a desktop was not created, or a process on it did not run to its end.
#[derive(Debug, thiserror::Error)]
pub enum DesktopError {
    #[error("the desktop name \"{0}\" is empty or holds a backslash")]
    BadName(String),
    #[error("could not create the desktop {name}")]
    Create { name: String, source: windows::core::Error },
    #[error("could not start {} on the desktop {name}", program.display())]
    Start { program: PathBuf, name: String, source: windows::core::Error },
    #[error(
        "could not put process {pid} on the desktop {name} in a job that ends with this process"
    )]
    Job { pid: u32, name: String, source: windows::core::Error },
    #[error("could not wait for process {pid} on the desktop {name}")]
    Wait { pid: u32, name: String, source: windows::core::Error },
    #[error("could not read the exit code of process {pid} on the desktop {name}")]
    ExitCode { pid: u32, name: String, source: windows::core::Error },
}

/// Checks that `name` can name a desktop: it is not empty and holds no backslash.
pub(crate) fn check_name(name: &str) -> Result<(), DesktopError> {
    if name.is_empty() || name.contains(STATION_SEPARATOR) {
        return Err(DesktopError::BadName(name.to_owned()));
    }
    Ok(())
}

/// The name of the desktop this process was started on, from its `STARTUPINFO`, without a window
/// station name before it. `None` when the process's start named no desktop.
pub fn startup_desktop() -> Option<String> {
    raw_desktop::startup_desktop().map(|full| desktop_part(&full).to_owned())
}

/// This process's entire command line as Windows holds it, the program name first.
pub fn own_command_line() -> OsString {
    raw_desktop::own_command_line()
}

/// The desktop name in `full`, which may lead with a window station name and a backslash.
fn desktop_part(full: &str) -> &str {
    full.rsplit_once(STATION_SEPARATOR).map_or(full, |(_station, desktop)| desktop)
}

#[cfg(test)]
mod tests {
    use super::{DesktopError, check_name, desktop_part};

    #[test]
    fn a_full_desktop_name_reads_as_its_desktop_part() {
        assert_eq!(desktop_part(r"WinSta0\Default"), "Default");
        assert_eq!(desktop_part("arcane-eye-trial-42"), "arcane-eye-trial-42");
        assert_eq!(desktop_part(r"WinSta0\"), "");
        assert_eq!(desktop_part(""), "");
    }

    #[test]
    fn an_empty_name_or_one_with_a_backslash_names_no_desktop() {
        check_name("arcane-eye-trial-42").expect("a plain name");
        for bad in ["", r"WinSta0\Default", "a\\"] {
            let error = check_name(bad).expect_err("a bad name");
            assert!(matches!(&error, DesktopError::BadName(name) if name == bad), "{error:?}");
        }
        let message = DesktopError::BadName(r"a\b".to_owned()).to_string();
        assert_eq!(message, "the desktop name \"a\\b\" is empty or holds a backslash");
    }
}
