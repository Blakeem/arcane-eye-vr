//! The game's mod check folder, which tells the main menu from the "Press any key" screen.
//!
//! The game creates the folder `ModCrashSanityCheck` in its profile folder when it starts, and
//! removes it when it shows the main menu. A session that loads the latest save from the "Press any
//! key" screen never shows the main menu, so it leaves the folder. The game refuses that load in a
//! session that found the folder at its start. A level trial moves the folder to the Recycle Bin
//! before its session, which costs no game start.

use std::io;
use std::path::{Path, PathBuf};

use diag::root::{self, RootError};
use launch::recycle::{self, RecycleError};

/// The game's profile folder in the local app data folder.
const PROFILE_FOLDER: &str = r"Larian Studios\Baldur's Gate 3";

/// The mod check folder's name in the profile folder.
const FOLDER: &str = "ModCrashSanityCheck";

/// Whether the game's mod check folder exists.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ModCheckFolder {
    Present,
    Absent,
}

/// Why the mod check folder could not be looked for or cleared.
#[derive(Debug, thiserror::Error)]
pub(crate) enum ModCheckError {
    #[error(transparent)]
    LocalAppData(#[from] RootError),
    #[error("could not look for the game's mod check folder {}", path.display())]
    Read { path: PathBuf, source: io::Error },
    #[error(transparent)]
    Recycle(#[from] RecycleError),
}

/// Whether the mod check folder exists in the current user's game profile folder.
pub(crate) fn read() -> Result<ModCheckFolder, ModCheckError> {
    read_in(&root::local_app_data()?)
}

/// Moves the mod check folder of the current user's game profile folder to the Recycle Bin. The
/// folder was empty in every read, and a folder with an entry stays.
pub(crate) fn clear() -> Result<(), ModCheckError> {
    Ok(recycle::recycle_empty_folder(&path_in(&root::local_app_data()?))?)
}

/// The mod check folder in the game's profile folder in `local_app_data`.
fn path_in(local_app_data: &Path) -> PathBuf {
    local_app_data.join(PROFILE_FOLDER).join(FOLDER)
}

/// Whether the mod check folder exists in the game's profile folder in `local_app_data`.
fn read_in(local_app_data: &Path) -> Result<ModCheckFolder, ModCheckError> {
    let path = path_in(local_app_data);
    match path.try_exists() {
        Ok(true) => Ok(ModCheckFolder::Present),
        Ok(false) => Ok(ModCheckFolder::Absent),
        Err(source) => Err(ModCheckError::Read { path, source }),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{ModCheckFolder, read_in};
    use crate::test_dir;

    #[test]
    fn the_folder_is_present_once_it_exists_in_the_games_profile_folder() {
        let dir = test_dir("mod-check-folder");
        let profile = dir.path().join("Larian Studios").join("Baldur's Gate 3");
        let read = || read_in(dir.path()).expect("a read");
        assert_eq!(read(), ModCheckFolder::Absent, "no profile folder");
        fs::create_dir_all(&profile).expect("create the profile folder");
        assert_eq!(read(), ModCheckFolder::Absent, "no mod check folder");
        fs::create_dir(profile.join("ModCrashSanityCheck")).expect("create the folder");
        assert_eq!(read(), ModCheckFolder::Present);
    }
}
