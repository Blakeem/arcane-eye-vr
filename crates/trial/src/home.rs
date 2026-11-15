//! The game's root folder of a trial, `home` in the result folder, and the settings files it gets.
//! The priming session of a level trial gets its own root folder, `prime`, with the same files.
//!
//! The game reads its settings and writes its session log there, so a trial never reads or changes
//! the player's root folder, and the result folder holds the settings the trial ran with.

use std::fs;
use std::io;
use std::path::{self, Path, PathBuf};

use settings::file::{DEV_FILE, SETTINGS_FILE};
use settings::rejection::Rejection;
use settings::{dev, player};

use crate::args::Settings;

/// The game's root folder's name in the result folder.
pub(crate) const HOME_FOLDER: &str = "home";

/// The priming session's root folder's name in the result folder.
pub(crate) const PRIME_FOLDER: &str = "prime";

/// Why a given settings file failed the check, or why the game's root folder was not filled.
#[derive(Debug, thiserror::Error)]
pub(crate) enum HomeError {
    #[error("the {kind} file {} is not an existing file", path.display())]
    Missing { kind: &'static str, path: PathBuf },
    #[error("could not read the {kind} file {}", path.display())]
    Read { kind: &'static str, path: PathBuf, source: io::Error },
    #[error("the {kind} file {} is rejected", path.display())]
    Rejected { kind: &'static str, path: PathBuf, source: Rejection },
    #[error("the {kind} file {} holds unknown keys: {}", path.display(), keys.join(", "))]
    UnknownKeys { kind: &'static str, path: PathBuf, keys: Vec<String> },
    #[error("could not make the game's root folder {} an absolute path", path.display())]
    Absolute { path: PathBuf, source: io::Error },
    #[error("could not create the game's root folder {}", path.display())]
    Create { path: PathBuf, source: io::Error },
    #[error("could not copy {} to {}", from.display(), to.display())]
    Copy { from: PathBuf, to: PathBuf, source: io::Error },
}

/// A settings file the command line may give: its kind, its name in the root folder, its path and
/// the parse that returns the path of each unknown key its text holds.
struct Given<'a> {
    kind: &'static str,
    name: &'static str,
    path: Option<&'a Path>,
    unknown_keys: fn(&str) -> Result<Vec<String>, Rejection>,
}

/// The settings files of `settings`, given or not.
fn given_files(settings: &Settings) -> [Given<'_>; 2] {
    [
        Given {
            kind: "settings",
            name: SETTINGS_FILE,
            path: settings.player_file.as_deref(),
            unknown_keys: player_unknown_keys,
        },
        Given {
            kind: "dev",
            name: DEV_FILE,
            path: settings.dev_file.as_deref(),
            unknown_keys: dev_unknown_keys,
        },
    ]
}

fn player_unknown_keys(text: &str) -> Result<Vec<String>, Rejection> {
    let parsed = player::parse(text)?;
    Ok(parsed.unknown_keys)
}

fn dev_unknown_keys(text: &str) -> Result<Vec<String>, Rejection> {
    // The dev parser rejects an unknown key, so a dev file it accepts holds none.
    dev::parse(text)?;
    Ok(Vec::new())
}

/// Checks that each settings file the command line gives is a file the game's core applies in full.
///
/// The core answers a rejected file or an unknown key with a warning in its log alone, so without
/// this check a trial would pass and record a setting that never applied.
pub(crate) fn check(settings: &Settings) -> Result<(), HomeError> {
    for given in given_files(settings) {
        if let Some(path) = given.path {
            check_file(&given, path)?;
        }
    }
    Ok(())
}

/// Checks that the `given` file at `path` exists, is read and parses with no unknown key, and returns
/// the text it checked.
fn check_file(given: &Given<'_>, path: &Path) -> Result<String, HomeError> {
    let kind = given.kind;
    // A folder gives no `NotFound` on read, so it is caught before the read.
    if path.is_dir() {
        return Err(HomeError::Missing { kind, path: path.to_path_buf() });
    }
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(source) if source.kind() == io::ErrorKind::NotFound => {
            return Err(HomeError::Missing { kind, path: path.to_path_buf() });
        }
        Err(source) => return Err(HomeError::Read { kind, path: path.to_path_buf(), source }),
    };
    match (given.unknown_keys)(&text) {
        Ok(keys) if keys.is_empty() => Ok(text),
        Ok(keys) => Err(HomeError::UnknownKeys { kind, path: path.to_path_buf(), keys }),
        Err(source) => Err(HomeError::Rejected { kind, path: path.to_path_buf(), source }),
    }
}

/// Creates the game's root folder `name` in the result folder `folder`, writes each given settings
/// file into it under its fixed name and returns the root folder as an absolute path.
///
/// Each file is checked again and its checked text is written, since the file may change after the
/// check step and the game must read the text that passed.
pub(crate) fn prepare(
    folder: &Path,
    name: &str,
    settings: &Settings,
) -> Result<PathBuf, HomeError> {
    let home = home_path(folder, name)?;
    if let Err(source) = fs::create_dir_all(&home) {
        return Err(HomeError::Create { path: home, source });
    }
    for given in given_files(settings) {
        if let Some(from) = given.path {
            let text = check_file(&given, from)?;
            let to = home.join(given.name);
            if let Err(source) = fs::write(&to, text) {
                return Err(HomeError::Copy { from: from.to_path_buf(), to, source });
            }
        }
    }
    Ok(home)
}

/// The game's root folder `name` in `folder` as an absolute path, as
/// [`HOME_VARIABLE`](ipc_layout::root_folder::HOME_VARIABLE) requires.
fn home_path(folder: &Path, name: &str) -> Result<PathBuf, HomeError> {
    let home = folder.join(name);
    match path::absolute(&home) {
        Ok(absolute) => Ok(absolute),
        Err(source) => Err(HomeError::Absolute { path: home, source }),
    }
}

#[cfg(test)]
mod tests {
    use std::env;
    use std::fs;
    use std::path::{Path, PathBuf};

    use settings::rejection::Position;

    use super::{HOME_FOLDER, HomeError, check, home_path};
    use crate::args::Settings;
    use crate::test_dir;

    fn given(player_file: Option<PathBuf>, dev_file: Option<PathBuf>) -> Settings {
        Settings { player_file, dev_file, ..Settings::default() }
    }

    /// The game's root folder `home` in `folder`, as the trial's own session gets it.
    fn prepare(folder: &Path, settings: &Settings) -> Result<PathBuf, HomeError> {
        super::prepare(folder, HOME_FOLDER, settings)
    }

    #[test]
    fn a_relative_result_folder_gives_an_absolute_home() {
        let folder = Path::new(r"trial-results\2026-11-13T143530Z-menu");
        let home = home_path(folder, HOME_FOLDER).expect("a path");
        let current = env::current_dir().expect("the current folder");
        assert_eq!(home, current.join(r"trial-results\2026-11-13T143530Z-menu\home"));
        assert!(home.is_absolute());
    }

    #[test]
    fn a_given_file_that_is_missing_or_a_folder_fails_the_check_and_is_named() {
        let dir = test_dir("home-check");
        let missing = dir.path().join("missing.toml");
        let error = check(&given(Some(missing.clone()), None)).expect_err("a missing file");
        assert!(
            matches!(&error, HomeError::Missing { kind: "settings", path } if *path == missing)
        );
        let expected = format!("the settings file {} is not an existing file", missing.display());
        assert_eq!(error.to_string(), expected);
        let folder = check(&given(None, Some(dir.path().to_path_buf()))).expect_err("a folder");
        assert!(matches!(folder, HomeError::Missing { kind: "dev", .. }));
        check(&given(None, None)).expect("no given file passes");
    }

    #[test]
    fn given_files_the_core_applies_in_full_pass_the_check() {
        let dir = test_dir("home-valid");
        let (player_file, dev_file) = (dir.path().join("ipd.toml"), dir.path().join("eye.toml"));
        fs::write(&player_file, "version = 1\n[stereo]\nipd_mm = 60\n").expect("write");
        fs::write(&dev_file, "[stereo]\nmain_view_eye = \"left\"\n").expect("write");
        check(&given(Some(player_file), Some(dev_file))).expect("both files apply in full");
    }

    #[test]
    fn a_given_file_the_core_rejects_fails_the_check_with_its_path_and_place() {
        let dir = test_dir("home-rejected");
        let dev_file = dir.path().join("eye.toml");
        fs::write(&dev_file, "[stereo]\nmain_view_eye = \"Left\"\n").expect("write");
        let error = check(&given(None, Some(dev_file.clone()))).expect_err("a wrong-case eye");
        let HomeError::Rejected { kind: "dev", path, source } = &error else {
            panic!("not a dev file rejection: {error:?}");
        };
        assert_eq!(*path, dev_file);
        assert_eq!(source.at, Some(Position { line: 2, column: 17 }));
        assert_eq!(error.to_string(), format!("the dev file {} is rejected", dev_file.display()));
    }

    #[test]
    fn an_unknown_key_in_a_given_settings_file_fails_the_check_and_is_named() {
        let dir = test_dir("home-unknown");
        let player_file = dir.path().join("ipd.toml");
        fs::write(&player_file, "[stereo]\nipd = 60\n").expect("write");
        let error = check(&given(Some(player_file.clone()), None)).expect_err("an unknown key");
        let expected =
            format!("the settings file {} holds unknown keys: stereo.ipd", player_file.display());
        assert_eq!(error.to_string(), expected);
    }

    #[test]
    fn a_given_file_that_is_not_text_is_a_read_error_that_names_it() {
        let dir = test_dir("home-read");
        let dev_file = dir.path().join("eye.toml");
        fs::write(&dev_file, b"\xff\xfe").expect("write bytes that are not UTF-8");
        let error = check(&given(None, Some(dev_file.clone()))).expect_err("not UTF-8 text");
        assert!(matches!(&error, HomeError::Read { kind: "dev", path, .. } if *path == dev_file));
        let expected = format!("could not read the dev file {}", dev_file.display());
        assert_eq!(error.to_string(), expected);
    }

    #[test]
    fn each_given_file_lands_in_home_under_its_fixed_name() {
        let dir = test_dir("home-fill");
        let (player_file, dev_file) = (dir.path().join("a.toml"), dir.path().join("b.toml"));
        fs::write(&player_file, "[stereo]\nipd_mm = 60\n").expect("write");
        fs::write(&dev_file, "[stereo]\nmain_view_eye = \"left\"\n").expect("write");
        let folder = dir.path().join("result");
        let home = prepare(&folder, &given(Some(player_file), Some(dev_file))).expect("home");
        assert_eq!(home, folder.join("home"));
        let read = |name: &str| fs::read_to_string(home.join(name)).expect("read a copied file");
        assert_eq!(read("settings.toml"), "[stereo]\nipd_mm = 60\n");
        assert_eq!(read("dev.toml"), "[stereo]\nmain_view_eye = \"left\"\n");
        let empty = prepare(&dir.path().join("empty"), &given(None, None)).expect("home");
        assert_eq!(fs::read_dir(&empty).expect("the empty home").count(), 0);
    }

    #[test]
    fn a_copy_that_fails_names_both_paths() {
        let dir = test_dir("home-copy");
        let dev_file = dir.path().join("eye.toml");
        fs::write(&dev_file, "[stereo]\nmain_view_eye = \"left\"\n").expect("write");
        let to = dir.path().join(r"result\home\dev.toml");
        fs::create_dir_all(&to).expect("a folder in the copy's place");
        let folder = dir.path().join("result");
        let error = prepare(&folder, &given(None, Some(dev_file.clone()))).expect_err("a folder");
        assert!(
            matches!(&error, HomeError::Copy { from, to: named, .. } if *from == dev_file && *named == to),
            "{error:?}"
        );
    }

    #[test]
    fn a_given_file_that_changes_after_the_check_is_checked_again_before_it_lands_in_home() {
        let dir = test_dir("home-recheck");
        let dev_file = dir.path().join("eye.toml");
        fs::write(&dev_file, "[stereo]\nmain_view_eye = \"left\"\n").expect("write");
        let settings = given(None, Some(dev_file.clone()));
        check(&settings).expect("the file passes the check step");
        fs::write(&dev_file, "[stereo]\nmain_view_eye = \"Left\"\n").expect("rewrite");
        let folder = dir.path().join("result");
        let error = prepare(&folder, &settings).expect_err("the changed file is rejected");
        assert!(
            matches!(&error, HomeError::Rejected { kind: "dev", path, .. } if *path == dev_file)
        );
        assert!(!folder.join(r"home\dev.toml").exists(), "a rejected file landed in home");
    }

    #[test]
    fn a_home_that_cannot_be_created_is_named() {
        let dir = test_dir("home-create");
        let home = dir.path().join("home");
        fs::write(&home, "a file in the root folder's place").expect("write");
        let error = prepare(dir.path(), &given(None, None)).expect_err("a file is not a folder");
        assert!(matches!(&error, HomeError::Create { path, .. } if *path == home), "{error:?}");
        let expected = format!("could not create the game's root folder {}", home.display());
        assert_eq!(error.to_string(), expected);
    }
}
