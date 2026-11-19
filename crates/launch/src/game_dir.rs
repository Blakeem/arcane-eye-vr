//! The game's install folder, found through Steam or given by the caller.
//!
//! Steam's folder is in the registry. Its library list names each library folder, and the library
//! that holds the game has the game's app manifest, which names the game's folder under
//! `steamapps\common\`.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::registry::{Hive, RegistryError, RegistryValue};
use crate::vdf;

/// The Steam client's folder, which the Steam installer writes.
const STEAM_PATH: RegistryValue =
    RegistryValue { hive: Hive::CurrentUser, key: r"Software\Valve\Steam", name: "SteamPath" };

const STEAMAPPS: &str = "steamapps";
const COMMON: &str = "common";
const LIBRARY_LIST: &str = "libraryfolders.vdf";
const MANIFEST: &str = concat!("appmanifest_", steam_app_id!(), ".acf");

const EXE_DIR: &str = "bin";
pub(crate) const EXE_NAME: &str = "bg3_dx11.exe";

/// Why the game's folder was not found.
#[derive(Debug, thiserror::Error)]
pub enum GameDirError {
    #[error("the registry holds no Steam folder at {0}")]
    NoSteam(RegistryValue),
    #[error(transparent)]
    SteamRead(#[from] RegistryError),
    #[error("could not read the Steam library list {path}")]
    LibraryList { path: PathBuf, source: io::Error },
    #[error("no library in the Steam library list {path} holds {MANIFEST}")]
    NoLibrary { path: PathBuf },
    #[error(
        "no library in the Steam library list {list} holds {MANIFEST}, and the check of {path} failed"
    )]
    LibraryCheck { list: PathBuf, path: PathBuf, source: io::Error },
    #[error("could not read the game's Steam manifest {path}")]
    Manifest { path: PathBuf, source: io::Error },
    #[error("the game's Steam manifest {path} names no install folder")]
    NoInstallDir { path: PathBuf },
    #[error("the game exe {path} does not exist or is not a file")]
    NoExe { path: PathBuf },
    #[error("could not check the game exe {path}")]
    ExeCheck { path: PathBuf, source: io::Error },
}

/// A folder that held the game exe when it was checked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GameDir(PathBuf);

impl GameDir {
    /// The game's folder at `path`. Checks only that the game exe exists, and reads no file.
    ///
    /// # Errors
    ///
    /// Returns [`GameDirError::NoExe`] when `path` holds no game exe, and
    /// [`GameDirError::ExeCheck`] when the check fails for another cause, such as access denied.
    pub fn new(path: PathBuf) -> Result<Self, GameDirError> {
        let exe = exe_in(&path);
        match is_file(&exe) {
            Ok(true) => Ok(Self(path)),
            Ok(false) => Err(GameDirError::NoExe { path: exe }),
            Err(source) => Err(GameDirError::ExeCheck { path: exe, source }),
        }
    }

    /// The game's folder in the Steam library that holds it.
    ///
    /// # Errors
    ///
    /// Returns the [`GameDirError`] variant of the step that failed: the Steam folder, the library
    /// list, the manifest or the game exe.
    pub fn find() -> Result<Self, GameDirError> {
        let steam = steam_dir(STEAM_PATH.read_string())?;
        Self::find_in(&steam)
    }

    pub fn as_path(&self) -> &Path {
        &self.0
    }

    pub fn exe(&self) -> PathBuf {
        exe_in(&self.0)
    }

    /// The folder that holds the game exe and the mod's two DLLs.
    pub fn bin(&self) -> PathBuf {
        self.0.join(EXE_DIR)
    }

    /// The game's folder in the libraries that the library list of the Steam folder `steam` names.
    fn find_in(steam: &Path) -> Result<Self, GameDirError> {
        let list_path = steam.join(STEAMAPPS).join(LIBRARY_LIST);
        let list = match fs::read_to_string(&list_path) {
            Ok(list) => list,
            Err(source) => return Err(GameDirError::LibraryList { path: list_path, source }),
        };
        let libraries = vdf::values(&list, "path")
            .into_iter()
            .map(|library| PathBuf::from(library).join(STEAMAPPS));
        let library = library_with_manifest(libraries, list_path)?;
        let manifest_path = library.join(MANIFEST);
        let manifest = match fs::read_to_string(&manifest_path) {
            Ok(manifest) => manifest,
            Err(source) => return Err(GameDirError::Manifest { path: manifest_path, source }),
        };
        let Some(install_dir) = vdf::values(&manifest, "installdir").into_iter().next() else {
            return Err(GameDirError::NoInstallDir { path: manifest_path });
        };
        Self::new(library.join(COMMON).join(install_dir))
    }
}

fn exe_in(game_dir: &Path) -> PathBuf {
    game_dir.join(EXE_DIR).join(EXE_NAME)
}

/// Whether `path` is a file. A missing entry is no file.
///
/// # Errors
///
/// Returns the I/O error of a check that fails for a cause other than a missing entry, such as
/// access denied.
pub fn is_file(path: &Path) -> io::Result<bool> {
    match fs::metadata(path) {
        Ok(metadata) => Ok(metadata.is_file()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

/// The first of `libraries`, the `steamapps` folders the library list `list` names, that holds the
/// game's manifest. A library whose check fails is passed over, since a list can name a drive that
/// is not ready, and its error names the cause when no library holds the manifest.
fn library_with_manifest(
    libraries: impl Iterator<Item = PathBuf>,
    list: PathBuf,
) -> Result<PathBuf, GameDirError> {
    let mut failed_check = None;
    for library in libraries {
        let manifest = library.join(MANIFEST);
        match is_file(&manifest) {
            Ok(true) => return Ok(library),
            Ok(false) => {}
            Err(source) => {
                failed_check.get_or_insert((manifest, source));
            }
        }
    }
    match failed_check {
        Some((path, source)) => Err(GameDirError::LibraryCheck { list, path, source }),
        None => Err(GameDirError::NoLibrary { path: list }),
    }
}

/// Steam's folder from `read`, the read of its registry value.
fn steam_dir(read: Result<Option<String>, RegistryError>) -> Result<PathBuf, GameDirError> {
    match read? {
        Some(path) => Ok(PathBuf::from(path)),
        None => Err(GameDirError::NoSteam(STEAM_PATH)),
    }
}

#[cfg(test)]
mod tests {
    use super::{GameDir, GameDirError, steam_dir};
    use crate::registry::{Hive, RegistryError, RegistryValue};
    use crate::test_dir;
    use std::fmt::Write as _;
    use std::fs;
    use std::path::{Path, PathBuf};

    const MANIFEST_TEXT: &str =
        "\"AppState\"\n{\n\t\"appid\"\t\t\"1086940\"\n\t\"installdir\"\t\t\"Baldurs Gate 3\"\n}\n";

    fn write(path: &Path, text: &str) {
        fs::create_dir_all(path.parent().expect("a parent folder")).expect("create the folder");
        fs::write(path, text).expect("write the file");
    }

    /// A library list in Steam's layout, with each path's backslashes doubled as Steam writes them.
    fn library_list(libraries: &[&Path]) -> String {
        let mut text = String::from("\"libraryfolders\"\n{\n");
        for (index, library) in libraries.iter().enumerate() {
            let escaped = library.to_str().expect("a UTF-8 path").replace('\\', r"\\");
            write!(
                text,
                "\t\"{index}\"\n\t{{\n\t\t\"path\"\t\t\"{escaped}\"\n\t\t\"label\"\t\t\"\"\n\t}}\n"
            )
            .expect("a String takes every write");
        }
        text.push_str("}\n");
        text
    }

    fn write_library_list(steam: &Path, libraries: &[&Path]) -> PathBuf {
        let list = steam.join(r"steamapps\libraryfolders.vdf");
        write(&list, &library_list(libraries));
        list
    }

    #[test]
    fn the_game_is_found_in_the_second_of_two_libraries() {
        let dir = test_dir("two-libraries");
        let root = dir.path();
        let (steam, second) = (root.join("Steam"), root.join(r"Second Library"));
        write_library_list(&steam, &[&steam, &second]);
        write(&second.join(r"steamapps\appmanifest_1086940.acf"), MANIFEST_TEXT);
        let game = second.join(r"steamapps\common\Baldurs Gate 3");
        write(&game.join(r"bin\bg3_dx11.exe"), "");
        let found = GameDir::find_in(&steam).expect("find the game");
        assert_eq!(found.as_path(), game);
        assert_eq!(found.exe(), game.join(r"bin\bg3_dx11.exe"));
    }

    #[test]
    fn a_library_path_with_doubled_backslashes_reads_as_a_windows_path() {
        let text = library_list(&[Path::new(r"D:\Games\Steam Library")]);
        assert!(text.contains(r#""D:\\Games\\Steam Library""#), "{text}");
        assert_eq!(crate::vdf::values(&text, "path"), [r"D:\Games\Steam Library"]);
    }

    #[test]
    fn a_missing_steam_value_is_an_error_that_names_it() {
        let error = steam_dir(Ok(None)).expect_err("no Steam folder");
        assert_eq!(
            error.to_string(),
            r"the registry holds no Steam folder at HKCU\Software\Valve\Steam\SteamPath"
        );
        let value = RegistryValue { hive: Hive::CurrentUser, key: "Software", name: "SteamPath" };
        let read = Err(RegistryError { value, source: windows::core::Error::empty() });
        assert!(matches!(steam_dir(read), Err(GameDirError::SteamRead(_))));
        assert_eq!(
            steam_dir(Ok(Some("c:/steam".to_owned()))).ok(),
            Some(PathBuf::from("c:/steam"))
        );
    }

    #[test]
    fn a_missing_library_list_is_an_error_that_names_its_path() {
        let dir = test_dir("no-list");
        let steam = dir.path();
        let error = GameDir::find_in(steam).expect_err("no library list");
        let list = steam.join(r"steamapps\libraryfolders.vdf");
        assert!(
            matches!(&error, GameDirError::LibraryList { path, .. } if *path == list),
            "{error:?}"
        );
        assert!(error.to_string().ends_with(&list.display().to_string()), "{error}");
    }

    #[test]
    fn a_list_with_no_library_that_holds_the_manifest_is_an_error_that_names_the_list() {
        let dir = test_dir("no-manifest");
        let root = dir.path();
        let steam = root.join("Steam");
        let list = write_library_list(&steam, &[&steam, &root.join("Other")]);
        let error = GameDir::find_in(&steam).expect_err("no manifest");
        assert!(matches!(&error, GameDirError::NoLibrary { path } if *path == list), "{error:?}");
        let expected = format!(
            "no library in the Steam library list {} holds appmanifest_1086940.acf",
            list.display()
        );
        assert_eq!(error.to_string(), expected);
    }

    #[test]
    fn a_library_that_cannot_be_checked_is_passed_over_for_the_next() {
        let dir = test_dir("library-check-passed");
        let root = dir.path();
        // Windows rejects `?` in a file name, so the check fails with an error other than not found.
        let (steam, bad) = (root.join("Steam"), root.join("bad?name"));
        write_library_list(&steam, &[&bad, &steam]);
        write(&steam.join(r"steamapps\appmanifest_1086940.acf"), MANIFEST_TEXT);
        let game = steam.join(r"steamapps\common\Baldurs Gate 3");
        write(&game.join(r"bin\bg3_dx11.exe"), "");
        let found = GameDir::find_in(&steam).expect("find the game in the second library");
        assert_eq!(found.as_path(), game);
    }

    #[test]
    fn a_library_that_cannot_be_checked_names_the_cause_when_no_library_holds_the_manifest() {
        let dir = test_dir("library-check-failed");
        let root = dir.path();
        let (steam, bad) = (root.join("Steam"), root.join("bad?name"));
        let list = write_library_list(&steam, &[&steam, &bad]);
        let error = GameDir::find_in(&steam).expect_err("no library holds the manifest");
        let manifest = bad.join(r"steamapps\appmanifest_1086940.acf");
        let GameDirError::LibraryCheck { list: error_list, path, source } = &error else {
            panic!("a failed check, not a missing manifest: {error:?}");
        };
        assert_eq!((error_list, path), (&list, &manifest));
        assert_ne!(source.kind(), std::io::ErrorKind::NotFound, "{source}");
        let expected = format!(
            "no library in the Steam library list {} holds appmanifest_1086940.acf, and the check \
             of {} failed",
            list.display(),
            manifest.display()
        );
        assert_eq!(error.to_string(), expected);
    }

    #[test]
    fn a_folder_without_the_game_exe_is_an_error_that_names_the_exe_path() {
        let dir = test_dir("no-exe");
        let steam = dir.path();
        write_library_list(steam, &[steam]);
        write(&steam.join(r"steamapps\appmanifest_1086940.acf"), MANIFEST_TEXT);
        let exe = steam.join(r"steamapps\common\Baldurs Gate 3\bin\bg3_dx11.exe");
        let error = GameDir::find_in(steam).expect_err("no game exe");
        assert!(matches!(&error, GameDirError::NoExe { path } if *path == exe), "{error:?}");
        let given = GameDir::new(steam.to_path_buf()).expect_err("no game exe in a given folder");
        let given_exe = steam.join(r"bin\bg3_dx11.exe");
        assert!(matches!(&given, GameDirError::NoExe { path } if *path == given_exe), "{given:?}");
        assert_eq!(
            given.to_string(),
            format!("the game exe {} does not exist or is not a file", given_exe.display())
        );
    }

    #[test]
    fn a_folder_named_as_the_game_exe_is_no_game_exe() {
        let dir = test_dir("exe-folder");
        let game = dir.path();
        let exe = game.join(r"bin\bg3_dx11.exe");
        fs::create_dir_all(&exe).expect("create a folder named as the exe");
        let error = GameDir::new(game.to_path_buf()).expect_err("a folder is no exe");
        assert!(matches!(&error, GameDirError::NoExe { path } if *path == exe), "{error:?}");
    }

    #[test]
    fn a_game_exe_that_cannot_be_checked_is_an_error_that_keeps_the_cause() {
        let dir = test_dir("exe-check");
        // Windows rejects `?` in a file name, so the check fails with an error other than not found.
        let game = dir.path().join("bad?name");
        let exe = game.join(r"bin\bg3_dx11.exe");
        let error = GameDir::new(game).expect_err("an invalid folder name");
        let GameDirError::ExeCheck { path, source } = &error else {
            panic!("a failed check, not a missing exe: {error:?}");
        };
        assert_eq!(*path, exe);
        assert_ne!(source.kind(), std::io::ErrorKind::NotFound, "{source}");
        assert_eq!(error.to_string(), format!("could not check the game exe {}", exe.display()));
    }

    #[test]
    fn a_manifest_without_an_install_folder_is_an_error_that_names_it() {
        let dir = test_dir("no-installdir");
        let steam = dir.path();
        write_library_list(steam, &[steam]);
        let manifest = steam.join(r"steamapps\appmanifest_1086940.acf");
        write(&manifest, "\"AppState\"\n{\n\t\"appid\"\t\t\"1086940\"\n}\n");
        let error = GameDir::find_in(steam).expect_err("no install folder");
        assert!(
            matches!(&error, GameDirError::NoInstallDir { path } if *path == manifest),
            "{error:?}"
        );
    }

    #[test]
    fn a_given_folder_with_the_game_exe_is_the_game_s_folder() {
        let dir = test_dir("given");
        let game = dir.path();
        write(&game.join(r"bin\bg3_dx11.exe"), "");
        let found = GameDir::new(game.to_path_buf()).expect("a game folder");
        assert_eq!(found.as_path(), game);
        assert_eq!(found.bin(), game.join("bin"));
    }
}
