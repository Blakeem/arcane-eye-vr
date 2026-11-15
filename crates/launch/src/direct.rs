//! The direct start: runs the game exe itself, so no Larian launcher and no Play click are needed.
//!
//! The command line and the working directory are the ones the Larian launcher gives the game. A
//! caller that asks for the latest save gets `-continueGame` after the launcher's arguments. The
//! two Steam variables keep the game from handing the start back to Steam. The root folder variable
//! gives the game's core the caller's root folder.

use std::io;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};

use ipc_layout::root_folder::HOME_VARIABLE;
use tracing::info;

use crate::game_dir::GameDir;

const GAME_ARGS: [&str; 5] = ["-externalcrashhandler", "-stats", "1", "-modded", "1"];

/// The game option that makes a key on the title screen load the latest save.
const CONTINUE_ARG: &str = "-continueGame";

/// Whether the game loads the latest save once a key moves its title screen on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Save {
    /// The key loads the latest save.
    LoadLatest,
    /// The key brings up the main menu.
    NoLoad,
}

/// The environment variables that tell the game Steam started it.
const STEAM_VARIABLES: [&str; 2] = ["SteamAppId", "SteamGameId"];

/// The Larian launcher's folder, the working directory it gives the game.
const WORKING_DIR: &str = "Launcher";

/// The game exe failed to start.
#[derive(Debug, thiserror::Error)]
#[error("could not start {program} in {working_dir}")]
pub struct StartError {
    pub program: PathBuf,
    pub working_dir: PathBuf,
    pub source: io::Error,
}

/// Starts the game exe in `game_dir` with `home` as its root folder, an absolute path as
/// [`HOME_VARIABLE`] requires, and with the save option `save`. The returned child holds the game's
/// process id and a handle to wait on.
///
/// # Errors
///
/// Returns [`StartError`] when Windows cannot start the game exe.
pub fn start(game_dir: &GameDir, home: &Path, save: Save) -> Result<Child, StartError> {
    let mut command = command(game_dir, home, save);
    match command.spawn() {
        Ok(child) => {
            info!(
                pid = child.id(),
                exe = %game_dir.exe().display(),
                home = %home.display(),
                save = ?save,
                "game started directly"
            );
            Ok(child)
        }
        Err(source) => {
            Err(StartError { program: game_dir.exe(), working_dir: working_dir(game_dir), source })
        }
    }
}

/// The command that starts the game exe in `game_dir` with the root folder `home` and the save
/// option `save`. The Steam variables and the root folder variable reach only the child.
fn command(game_dir: &GameDir, home: &Path, save: Save) -> Command {
    let mut command = Command::new(game_dir.exe());
    command.args(GAME_ARGS).current_dir(working_dir(game_dir));
    if save == Save::LoadLatest {
        command.arg(CONTINUE_ARG);
    }
    for name in STEAM_VARIABLES {
        command.env(name, steam_app_id!());
    }
    command.env(HOME_VARIABLE, home);
    command
}

fn working_dir(game_dir: &GameDir) -> PathBuf {
    game_dir.as_path().join(WORKING_DIR)
}

#[cfg(test)]
mod tests {
    use super::{Save, command, start};
    use crate::game_dir::GameDir;
    use crate::test_dir;
    use std::ffi::OsStr;
    use std::fs;
    use std::path::Path;
    use test_support::TestDir;

    /// A game folder whose exe is an empty file, which Windows refuses to run.
    fn stand_in_game(name: &str) -> TestDir {
        let dir = test_dir(name);
        fs::create_dir_all(dir.path().join("bin")).expect("create the game folder");
        fs::write(dir.path().join(r"bin\bg3_dx11.exe"), "").expect("write a stand-in exe");
        dir
    }

    #[test]
    fn the_command_runs_the_game_exe_with_the_launcher_s_arguments_and_the_added_variables() {
        let dir = stand_in_game("direct");
        let game = dir.path();
        let home = Path::new(r"D:\trial-results\2026-11-13T143530Z-menu\home");
        let game_dir = GameDir::new(game.to_path_buf()).expect("a game folder");
        let command = command(&game_dir, home, Save::NoLoad);
        assert_eq!(command.get_program(), game.join(r"bin\bg3_dx11.exe"));
        let args: Vec<&OsStr> = command.get_args().collect();
        assert_eq!(args, ["-externalcrashhandler", "-stats", "1", "-modded", "1"]);
        let envs: Vec<(&OsStr, Option<&OsStr>)> = command.get_envs().collect();
        let app_id = Some(OsStr::new("1086940"));
        let expected = [
            (OsStr::new("ARCANE_EYE_VR_HOME"), Some(home.as_os_str())),
            (OsStr::new("SteamAppId"), app_id),
            (OsStr::new("SteamGameId"), app_id),
        ];
        assert_eq!(envs, expected);
        assert_eq!(command.get_current_dir(), Some(game.join("Launcher").as_path()));
    }

    #[test]
    fn the_command_that_loads_the_latest_save_ends_with_the_continue_option() {
        let dir = stand_in_game("direct-continue");
        let game_dir = GameDir::new(dir.path().to_path_buf()).expect("a game folder");
        let command = command(&game_dir, Path::new(r"D:\home"), Save::LoadLatest);
        let args: Vec<&OsStr> = command.get_args().collect();
        let expected = ["-externalcrashhandler", "-stats", "1", "-modded", "1", "-continueGame"];
        assert_eq!(args, expected);
    }

    #[test]
    fn a_start_that_fails_names_the_program_and_the_working_directory() {
        let dir = stand_in_game("direct-fails");
        let game = dir.path();
        let game_dir = GameDir::new(game.to_path_buf()).expect("a game folder");
        let error = start(&game_dir, &game.join("home"), Save::NoLoad)
            .expect_err("an empty exe in a folder with no Launcher folder does not start");
        assert_eq!(error.program, game.join(r"bin\bg3_dx11.exe"));
        assert_eq!(error.working_dir, game.join("Launcher"));
        let expected = format!(
            "could not start {} in {}",
            game.join(r"bin\bg3_dx11.exe").display(),
            game.join("Launcher").display()
        );
        assert_eq!(error.to_string(), expected);
    }
}
