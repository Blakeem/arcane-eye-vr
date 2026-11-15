//! The watch of the player file and the dev file in one folder: which file changed since the check
//! before.
//!
//! A file's stamp is its modification time and its length. A check takes each file's stamp, then
//! reads and parses the file when the stamp differs from the one the check before took. So the first
//! check reads both files, and a file written during a read is read again at the next check.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::dev::DevFlags;
use crate::file::{self, DEV_FILE, FileError, SETTINGS_FILE};
use crate::player::Parsed;

/// What tells one state of a file from another.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stamp {
    Missing,
    Present {
        modified: SystemTime,
        len: u64,
    },
    /// The file's metadata could not be read, with the kind of the error.
    Unreadable(io::ErrorKind),
}

impl Stamp {
    fn of(path: &Path) -> Self {
        let metadata = match fs::metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Self::Missing,
            Err(error) => return Self::Unreadable(error.kind()),
        };
        match metadata.modified() {
            Ok(modified) => Self::Present { modified, len: metadata.len() },
            Err(error) => Self::Unreadable(error.kind()),
        }
    }
}

/// What one check found. A field is `None` for a file whose stamp did not change.
#[derive(Debug, Default)]
pub struct Changes {
    /// The read of a changed `settings.toml`.
    pub settings: Option<Result<Parsed, FileError>>,
    /// The read of a changed `dev.toml`.
    pub dev: Option<Result<DevFlags, FileError>>,
}

/// The watch of both files in one folder.
#[derive(Debug)]
pub struct Watch {
    folder: PathBuf,
    /// The stamp each check before took, or `None` before the first check.
    settings: Option<Stamp>,
    dev: Option<Stamp>,
}

impl Watch {
    /// A watch of the files in `folder` that has checked nothing yet.
    pub fn new(folder: &Path) -> Self {
        Self { folder: folder.to_path_buf(), settings: None, dev: None }
    }

    /// Reads each file whose stamp differs from the stamp of the check before. The first check reads
    /// both files.
    pub fn check(&mut self) -> Changes {
        let settings = changed(&self.folder.join(SETTINGS_FILE), &mut self.settings);
        let dev = changed(&self.folder.join(DEV_FILE), &mut self.dev);
        Changes {
            settings: settings.then(|| file::read_settings(&self.folder)),
            dev: dev.then(|| file::read_dev(&self.folder)),
        }
    }
}

/// Whether the stamp of the file at `path` differs from `last`, which then holds the new stamp.
fn changed(path: &Path, last: &mut Option<Stamp>) -> bool {
    let stamp = Stamp::of(path);
    let differs = *last != Some(stamp);
    *last = Some(stamp);
    differs
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{Changes, Watch};
    use crate::dev::{DevFlags, MainViewEye};
    use crate::file::FileError;
    use crate::player::{IpdMm, Parsed};
    use crate::test_dir;

    fn ipd(changes: &Changes) -> Option<f32> {
        match &changes.settings {
            Some(Ok(parsed)) => Some(parsed.settings.stereo.ipd_mm.millimeters()),
            Some(Err(_)) | None => None,
        }
    }

    #[test]
    fn the_first_check_reads_both_files_and_the_next_check_reads_none() {
        let dir = test_dir("watch-first");
        fs::write(dir.path().join("settings.toml"), "[stereo]\nipd_mm = 58\n").expect("write");
        let mut watch = Watch::new(dir.path());
        let first = watch.check();
        assert_eq!(ipd(&first), Some(58.0));
        assert_eq!(first.dev.expect("a first read").expect("defaults"), DevFlags::default());
        let second = watch.check();
        assert!(second.settings.is_none() && second.dev.is_none(), "{second:?}");
    }

    #[test]
    fn missing_files_give_the_defaults_once() {
        let dir = test_dir("watch-missing");
        let mut watch = Watch::new(&dir.path().join("no-folder"));
        let first = watch.check();
        assert_eq!(first.settings.expect("a first read").expect("defaults"), Parsed::default());
        assert_eq!(first.dev.expect("a first read").expect("defaults"), DevFlags::default());
        let second = watch.check();
        assert!(second.settings.is_none() && second.dev.is_none(), "{second:?}");
    }

    #[test]
    fn a_changed_file_is_read_again_and_the_other_file_is_not() {
        let dir = test_dir("watch-changed");
        let settings = dir.path().join("settings.toml");
        fs::write(&settings, "[stereo]\nipd_mm = 58\n").expect("write");
        let mut watch = Watch::new(dir.path());
        assert_eq!(ipd(&watch.check()), Some(58.0));
        // Each text has another length, so the stamp changes even when the file time does not.
        fs::write(&settings, "[stereo]\nipd_mm = 71.5\n").expect("write");
        let changed = watch.check();
        assert_eq!(ipd(&changed), Some(71.5));
        assert!(changed.dev.is_none(), "the dev file did not change");
        fs::write(dir.path().join("dev.toml"), "[stereo]\nmain_view_eye = \"left\"\n")
            .expect("write");
        let dev = watch.check();
        assert!(dev.settings.is_none(), "the player file did not change");
        let flags = dev.dev.expect("a changed dev file").expect("valid flags");
        assert_eq!(flags.stereo.main_view_eye, MainViewEye::Left);
    }

    #[test]
    fn a_rejected_change_is_reported_once_and_a_removed_file_gives_the_defaults() {
        let dir = test_dir("watch-rejected");
        let settings = dir.path().join("settings.toml");
        fs::write(&settings, "[stereo]\nipd_mm = 58\n").expect("write");
        let mut watch = Watch::new(dir.path());
        assert_eq!(ipd(&watch.check()), Some(58.0));
        fs::write(&settings, "[stereo]\nipd_mm = 500\n").expect("write");
        let rejected = watch.check().settings.expect("a changed file");
        assert!(matches!(rejected, Err(FileError::Rejected { .. })), "{rejected:?}");
        assert!(watch.check().settings.is_none(), "the rejected text is not read again");
        fs::remove_file(&settings).expect("remove");
        let removed = watch.check().settings.expect("a removed file").expect("defaults");
        assert_eq!(removed.settings.stereo.ipd_mm, IpdMm::default());
    }
}
