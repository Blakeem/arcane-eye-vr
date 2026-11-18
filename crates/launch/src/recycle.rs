//! The move of an empty folder to the Recycle Bin.
//!
//! The trial runner clears the game's mod check folder this way, so the player can see in the
//! Recycle Bin what a trial removed. On a volume with no Recycle Bin the shell removes the folder for
//! good. The folder is empty, so nothing is lost there.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::raw::recycle::{Moved, to_recycle_bin};

/// Why a folder did not go to the Recycle Bin.
#[derive(Debug, thiserror::Error)]
pub enum RecycleError {
    #[error("{} is not an absolute path", path.display())]
    NotAbsolute { path: PathBuf },
    #[error("could not read the folder {}", path.display())]
    Read { path: PathBuf, source: io::Error },
    #[error("the folder {} is not empty", path.display())]
    NotEmpty { path: PathBuf },
    #[error("the shell's move of {} to the Recycle Bin failed with code {code:#x}", path.display())]
    Shell { path: PathBuf, code: i32 },
    #[error("the shell stopped its move of {} to the Recycle Bin", path.display())]
    Aborted { path: PathBuf },
    #[error("the folder {} still exists after its move to the Recycle Bin", path.display())]
    StillThere { path: PathBuf },
}

/// Checks that `folder` is an absolute path to a folder with no entry.
///
/// # Errors
///
/// Returns the check that failed.
pub fn check_empty_folder(folder: &Path) -> Result<(), RecycleError> {
    let path = || folder.to_path_buf();
    if !folder.is_absolute() {
        return Err(RecycleError::NotAbsolute { path: path() });
    }
    let mut entries = match fs::read_dir(folder) {
        Ok(entries) => entries,
        Err(source) => return Err(RecycleError::Read { path: path(), source }),
    };
    match entries.next() {
        None => Ok(()),
        Some(Ok(_)) => Err(RecycleError::NotEmpty { path: path() }),
        Some(Err(source)) => Err(RecycleError::Read { path: path(), source }),
    }
}

/// Moves `folder`, an absolute path to a folder with no entry, to the Recycle Bin.
///
/// # Errors
///
/// Returns the check that failed before the move, the shell's failure, or
/// [`RecycleError::StillThere`] when the folder exists after the move.
pub fn recycle_empty_folder(folder: &Path) -> Result<(), RecycleError> {
    let path = || folder.to_path_buf();
    check_empty_folder(folder)?;
    match to_recycle_bin(folder) {
        Moved::Done => {}
        Moved::Aborted => return Err(RecycleError::Aborted { path: path() }),
        Moved::Failed(code) => return Err(RecycleError::Shell { path: path(), code }),
    }
    match folder.try_exists() {
        Ok(false) => Ok(()),
        Ok(true) => Err(RecycleError::StillThere { path: path() }),
        Err(source) => Err(RecycleError::Read { path: path(), source }),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use super::{RecycleError, check_empty_folder, recycle_empty_folder};
    use crate::test_dir;

    #[test]
    fn a_folder_with_an_entry_a_missing_folder_and_a_relative_path_are_refused() {
        let dir = test_dir("recycle-refused");
        let folder = dir.path().join("check");
        let missing = check_empty_folder(&folder);
        assert!(matches!(missing, Err(RecycleError::Read { .. })), "{missing:?}");
        fs::create_dir(&folder).expect("create the folder");
        check_empty_folder(&folder).expect("an empty folder passes");
        fs::write(folder.join("kept.txt"), "kept").expect("write a file");
        let filled = recycle_empty_folder(&folder);
        assert!(matches!(filled, Err(RecycleError::NotEmpty { .. })), "{filled:?}");
        assert!(folder.join("kept.txt").exists(), "a refused folder keeps its file");
        let relative = recycle_empty_folder(Path::new("check"));
        assert!(matches!(relative, Err(RecycleError::NotAbsolute { .. })), "{relative:?}");
    }

    #[test]
    #[ignore = "it leaves a folder in the Recycle Bin of the user who runs it"]
    fn an_empty_folder_goes_to_the_recycle_bin() {
        let dir = test_dir("recycle-empty");
        let folder = dir.path().join("arcane-eye-recycle-test");
        fs::create_dir(&folder).expect("create the folder");
        recycle_empty_folder(&folder).expect("the move");
        assert!(!folder.exists());
    }
}
