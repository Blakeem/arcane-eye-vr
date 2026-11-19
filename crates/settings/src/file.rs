//! Reads the player file and the dev file from the folder the caller names, and writes the table
//! `[screen]` into the player file.
//!
//! A missing file holds no overrides, as an empty file does, so it gives the defaults.

use std::fs::{self, File};
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};

use crate::dev::{self, DevFlags};
use crate::edit::{self, EditError};
use crate::player::{self, ScreenSettings};
use crate::rejection::Rejection;

/// The player file's name in the mod's root folder.
pub const SETTINGS_FILE: &str = "settings.toml";

/// The dev file's name in the mod's root folder.
pub const DEV_FILE: &str = "dev.toml";

/// The file a write of the player file fills before it replaces the player file. A failed write
/// may leave it, and the next write overwrites it.
const SETTINGS_FILE_NEW: &str = "settings.toml.new";

/// Why a file gave no values.
#[derive(Debug, thiserror::Error)]
pub enum FileError {
    #[error("could not read {}", path.display())]
    Read { path: PathBuf, source: io::Error },
    #[error("{} is rejected", path.display())]
    Rejected { path: PathBuf, source: Rejection },
}

/// Why the player file was not written. The file is as it was before the write.
#[derive(Debug, thiserror::Error)]
pub enum WriteError {
    #[error("could not create the folder {}", path.display())]
    CreateFolder { path: PathBuf, source: io::Error },
    #[error("could not read {}", path.display())]
    Read { path: PathBuf, source: io::Error },
    #[error("{} is rejected", path.display())]
    Rejected { path: PathBuf, source: EditError },
    #[error("could not write {}", path.display())]
    Write { path: PathBuf, source: io::Error },
    #[error("could not replace {} with {}", path.display(), new_path.display())]
    Replace { path: PathBuf, new_path: PathBuf, source: io::Error },
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

/// Sets the keys `distance_m` and `width_m` of the table `[screen]` of `settings.toml` in `folder`
/// to `screen`, and keeps the content of every other line of the file. Each line of the written
/// file ends as the first line ends. A missing file or folder is created.
///
/// # Errors
///
/// Returns [`WriteError::Rejected`] when the file is not valid TOML or its key `screen` is not a
/// table, and the variant of the step that failed otherwise.
pub fn write_screen(folder: &Path, screen: ScreenSettings) -> Result<(), WriteError> {
    let path = folder.join(SETTINGS_FILE);
    let new_path = folder.join(SETTINGS_FILE_NEW);
    if let Err(source) = fs::create_dir_all(folder) {
        return Err(WriteError::CreateFolder { path: folder.to_path_buf(), source });
    }
    let text = match existing_text(&path) {
        Ok(Some(text)) => text,
        Ok(None) => edit::new_file_text(),
        Err(source) => return Err(WriteError::Read { path, source }),
    };
    let edited = match edit::screen_text(&text, screen) {
        Ok(edited) => edited,
        Err(source) => return Err(WriteError::Rejected { path, source }),
    };
    // The new file holds the entire text on the disk before it replaces the old one, so a failure
    // at any step leaves the old file whole.
    if let Err(source) = write_synced(&new_path, &edited) {
        return Err(WriteError::Write { path: new_path, source });
    }
    match fs::rename(&new_path, &path) {
        Ok(()) => Ok(()),
        Err(source) => Err(WriteError::Replace { path, new_path, source }),
    }
}

/// The text of the file at `path`, or no text when the file is missing.
fn read_text(path: &Path) -> Result<String, FileError> {
    match existing_text(path) {
        Ok(text) => Ok(text.unwrap_or_default()),
        Err(source) => Err(FileError::Read { path: path.to_path_buf(), source }),
    }
}

/// The text of the file at `path`, or `None` when the file is missing.
fn existing_text(path: &Path) -> io::Result<Option<String>> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

/// Writes `text` to the file at `path`, created or emptied first, and waits until the disk holds it.
fn write_synced(path: &Path, text: &str) -> io::Result<()> {
    let mut file = File::create(path)?;
    file.write_all(text.as_bytes())?;
    file.sync_all()
}

#[cfg(test)]
mod tests {
    use std::fs;

    use std::fs::OpenOptions;
    use std::os::windows::fs::OpenOptionsExt as _;

    use super::{FileError, WriteError, read_dev, read_settings, write_screen};
    use crate::dev::{DevFlags, MainViewEye};
    use crate::player::{IpdMm, Parsed, ScreenDistanceM, ScreenSettings, ScreenWidthM, Settings};
    use crate::rejection::Position;
    use crate::test_dir;

    fn screen(distance_m: f32, width_m: f32) -> ScreenSettings {
        ScreenSettings {
            distance_m: ScreenDistanceM::try_from(distance_m).expect("a distance in range"),
            width_m: ScreenWidthM::try_from(width_m).expect("a width in range"),
        }
    }

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

    #[test]
    fn a_written_screen_is_read_back_and_every_other_byte_stays() {
        let dir = test_dir("write-existing");
        let path = dir.path().join("settings.toml");
        let text = "# mine\nversion = 1\n\n[stereo]\nipd_mm = 58 # measured\n\n[screen]\n\
            distance_m = 4.0\nwidth_m = 2.0\n\n# the end\n";
        fs::write(&path, text).expect("write");
        write_screen(dir.path(), screen(2.449_999, 3.1)).expect("a valid file");
        let expected = "# mine\nversion = 1\n\n[stereo]\nipd_mm = 58 # measured\n\n[screen]\n\
            distance_m = 2.45\nwidth_m = 3.1\n\n# the end\n";
        assert_eq!(fs::read_to_string(&path).expect("read"), expected);
        let settings = read_settings(dir.path()).expect("the written file").settings;
        assert_eq!(settings.screen, screen(2.45, 3.1));
        assert_eq!(settings.stereo.ipd_mm, IpdMm::try_from(58.0).expect("in range"));
        assert!(!dir.path().join("settings.toml.new").exists(), "the new file replaced the old");
    }

    #[test]
    fn a_missing_file_and_folder_are_created_with_the_version_and_the_screen() {
        let dir = test_dir("write-missing");
        let folder = dir.path().join("no-folder");
        write_screen(&folder, screen(2.5, 3.0)).expect("a missing file");
        let written = fs::read_to_string(folder.join("settings.toml")).expect("the created file");
        assert_eq!(written, "version = 1\n\n[screen]\ndistance_m = 2.5\nwidth_m = 3.0\n");
        let parsed = read_settings(&folder).expect("the created file");
        assert_eq!(parsed.settings.screen, screen(2.5, 3.0));
        assert_eq!(parsed.settings.stereo, Settings::default().stereo);
        assert_eq!(parsed.unknown_keys, [""; 0]);
    }

    #[test]
    fn a_rejected_file_names_why_and_keeps_its_bytes() {
        let dir = test_dir("write-rejected");
        let path = dir.path().join("settings.toml");
        for (text, why) in [
            ("[stereo\nipd_mm = 58\n", "not valid TOML: "),
            ("screen = 3\n", "screen must be a table, got integer"),
        ] {
            fs::write(&path, text).expect("write");
            let error = write_screen(dir.path(), screen(2.5, 3.0)).expect_err(text);
            let WriteError::Rejected { path: named, source } = &error else {
                panic!("not a rejection: {error:?}");
            };
            assert_eq!(*named, path);
            assert!(source.to_string().starts_with(why), "{source}");
            assert_eq!(error.to_string(), format!("{} is rejected", path.display()));
            assert_eq!(fs::read_to_string(&path).expect("read"), text);
        }
    }

    #[test]
    fn a_file_that_cannot_be_read_is_not_written() {
        let dir = test_dir("write-unreadable");
        let path = dir.path().join("settings.toml");
        fs::create_dir(&path).expect("a folder in the file's place");
        let error = write_screen(dir.path(), screen(2.5, 3.0)).expect_err("a folder is not a file");
        assert!(matches!(&error, WriteError::Read { path: named, .. } if *named == path));
        assert!(path.is_dir(), "the folder stays");
    }

    #[test]
    fn a_folder_that_cannot_be_created_is_an_error_that_names_it() {
        let dir = test_dir("write-no-folder");
        let file = dir.path().join("a-file");
        fs::write(&file, "").expect("write");
        let folder = file.join("folder");
        let error = write_screen(&folder, screen(2.5, 3.0)).expect_err("a file is not a folder");
        assert!(matches!(&error, WriteError::CreateFolder { path: named, .. } if *named == folder));
        assert_eq!(fs::read_to_string(&file).expect("read"), "");
    }

    #[test]
    fn a_failed_write_of_the_new_file_leaves_the_old_file_whole() {
        let dir = test_dir("write-failed");
        let path = dir.path().join("settings.toml");
        let new_path = dir.path().join("settings.toml.new");
        fs::write(&path, "version = 1\n").expect("write");
        fs::create_dir(&new_path).expect("a folder in the new file's place");
        let error = write_screen(dir.path(), screen(2.5, 3.0)).expect_err("the new file fails");
        assert!(matches!(&error, WriteError::Write { path: named, .. } if *named == new_path));
        assert_eq!(fs::read_to_string(&path).expect("read"), "version = 1\n");
    }

    #[test]
    fn a_failed_replace_leaves_the_old_file_whole() {
        // FILE_SHARE_READ alone: the open file refuses the rename that would replace it.
        const SHARE_READ: u32 = 1;
        let dir = test_dir("write-replace");
        let path = dir.path().join("settings.toml");
        fs::write(&path, "version = 1\n").expect("write");
        let held = OpenOptions::new().read(true).share_mode(SHARE_READ).open(&path).expect("open");
        let error = write_screen(dir.path(), screen(2.5, 3.0)).expect_err("the file is held");
        drop(held);
        assert!(matches!(&error, WriteError::Replace { path: named, .. } if *named == path));
        assert_eq!(fs::read_to_string(&path).expect("read"), "version = 1\n");
    }

    #[test]
    fn a_stale_new_file_is_overwritten() {
        let dir = test_dir("write-stale");
        fs::write(dir.path().join("settings.toml.new"), "left by a failed write").expect("write");
        write_screen(dir.path(), screen(2.5, 3.0)).expect("a stale new file");
        let settings = read_settings(dir.path()).expect("the written file").settings;
        assert_eq!(settings.screen, screen(2.5, 3.0));
    }
}
