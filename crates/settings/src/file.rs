//! Reads the player file and the dev file from the folder the caller names.
//!
//! A missing file holds no overrides, as an empty file does, so it gives the defaults.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::dev::{self, DevFlags};
use crate::player;
use crate::rejection::Rejection;

/// The player file's name in the mod's root folder.
pub const SETTINGS_FILE: &str = "settings.toml";

/// The dev file's name in the mod's root folder.
pub const DEV_FILE: &str = "dev.toml";

/// Why a file gave no values.
#[derive(Debug, thiserror::Error)]
pub enum FileError {
    #[error("could not read {}", path.display())]
    Read { path: PathBuf, source: io::Error },
    #[error("{} is rejected", path.display())]
    Rejected { path: PathBuf, source: Rejection },
}

/// Reads and parses `settings.toml` in `folder`.
///
/// # Errors
///
/// Returns [`FileError::Read`] when the file exists and cannot be read as UTF-8 text, and
/// [`FileError::Rejected`] when its text is rejected.
pub fn read_settings(folder: &Path) -> Result<player::Parsed, FileError> {
    let path = folder.join(SETTINGS_FILE);
    let text = read_text(&path)?;
    match player::parse(&text) {
        Ok(parsed) => Ok(parsed),
        Err(source) => Err(FileError::Rejected { path, source }),
    }
}

/// Reads and parses `dev.toml` in `folder`.
///
/// # Errors
///
/// Returns [`FileError::Read`] when the file exists and cannot be read as UTF-8 text, and
/// [`FileError::Rejected`] when its text is rejected.
pub fn read_dev(folder: &Path) -> Result<DevFlags, FileError> {
    let path = folder.join(DEV_FILE);
    let text = read_text(&path)?;
    match dev::parse(&text) {
        Ok(flags) => Ok(flags),
        Err(source) => Err(FileError::Rejected { path, source }),
    }
}

/// The text of the file at `path`, or no text when the file is missing.
fn read_text(path: &Path) -> Result<String, FileError> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(text),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(String::new()),
        Err(source) => Err(FileError::Read { path: path.to_path_buf(), source }),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{FileError, read_dev, read_settings};
    use crate::dev::{DevFlags, MainViewEye};
    use crate::player::{IpdMm, Parsed};
    use crate::rejection::Position;
    use crate::test_dir;

    #[test]
    fn missing_files_and_a_missing_folder_give_the_defaults() {
        let dir = test_dir("missing");
        assert_eq!(read_settings(dir.path()).expect("defaults"), Parsed::default());
        assert_eq!(read_dev(dir.path()).expect("defaults"), DevFlags::default());
        let no_folder = dir.path().join("no-folder");
        assert_eq!(read_settings(&no_folder).expect("defaults"), Parsed::default());
        assert_eq!(read_dev(&no_folder).expect("defaults"), DevFlags::default());
    }

    #[test]
    fn each_file_is_read_under_its_fixed_name() {
        let dir = test_dir("valid");
        fs::write(dir.path().join("settings.toml"), "[stereo]\nipd_mm = 58\n").expect("write");
        fs::write(dir.path().join("dev.toml"), "[stereo]\nmain_view_eye = \"right\"\n")
            .expect("write");
        let settings = read_settings(dir.path()).expect("a valid settings file").settings;
        assert_eq!(settings.stereo.ipd_mm, IpdMm::try_from(58.0).expect("in range"));
        let flags = read_dev(dir.path()).expect("a valid dev file");
        assert_eq!(flags.stereo.main_view_eye, MainViewEye::Right);
    }

    #[test]
    fn a_rejected_file_names_its_path_and_the_rejection() {
        let dir = test_dir("rejected");
        let path = dir.path().join("dev.toml");
        fs::write(&path, "[stereo]\nmain_view_eye = \"up\"\n").expect("write");
        let error = read_dev(dir.path()).expect_err("an unknown eye");
        let FileError::Rejected { path: named, source } = &error else {
            panic!("not a rejection: {error:?}");
        };
        assert_eq!(*named, path);
        assert_eq!(source.at, Some(Position { line: 2, column: 17 }));
        assert_eq!(error.to_string(), format!("{} is rejected", path.display()));
    }

    #[test]
    fn a_file_that_cannot_be_read_is_a_read_error_that_names_it() {
        let dir = test_dir("unreadable");
        let path = dir.path().join("settings.toml");
        fs::create_dir(&path).expect("a folder in the file's place");
        fs::write(dir.path().join("dev.toml"), b"\xff\xfe")
            .expect("write bytes that are not UTF-8");
        let folder_error = read_settings(dir.path()).expect_err("a folder is not a file");
        assert!(matches!(&folder_error, FileError::Read { path: named, .. } if *named == path));
        assert_eq!(folder_error.to_string(), format!("could not read {}", path.display()));
        let bytes_error = read_dev(dir.path()).expect_err("bytes that are not UTF-8");
        let dev_path = dir.path().join("dev.toml");
        assert!(matches!(&bytes_error, FileError::Read { path: named, .. } if *named == dev_path));
    }
}
