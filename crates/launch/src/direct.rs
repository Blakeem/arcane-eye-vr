//! The direct start: runs the game exe itself, so no Larian launcher and no Play click are needed.
//!
//! The command line and the working directory are the ones the Larian launcher gives the game. The
//! two Steam variables keep the game from handing the start back to Steam. The root folder variable
//! gives the game's core the caller's root folder.

use std::io;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};

use ipc_layout::root_folder::HOME_VARIABLE;
use tracing::info;

use crate::game_dir::GameDir;

const GAME_ARGS: [&str; 5] = ["-externalcrashhandler", "-stats", "1", "-modded", "1"];

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
/// [`HOME_VARIABLE`] requires. The returned child holds the game's process id and a handle to wait
/// on.
///
/// # Errors
///
/// Returns [`StartError`] when Windows cannot start the game exe.
pub fn start(game_dir: &GameDir, home: &Path) -> Result<Child, StartError> {
    let mut command = command(game_dir, home);
    match command.spawn() {
        Ok(child) => {
            info!(
                pid = child.id(),
                exe = %game_dir.exe().display(),
                home = %home.display(),
                "game started directly"
            );
            Ok(child)
        }
        Err(source) => {
            Err(StartError { program: game_dir.exe(), working_dir: working_dir(game_dir), source })
        }
    }
}

/// The command that starts the game exe in `game_dir` with the root folder `home`. The Steam
/// variables and the root folder variable reach only the child.
fn command(game_dir: &GameDir, home: &Path) -> Command {
    let mut command = Command::new(game_dir.exe());
    command.args(GAME_ARGS).current_dir(working_dir(game_dir));
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
    use super::{command, start};
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
        let command = command(&GameDir::new(game.to_path_buf()).expect("a game folder"), home);
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
    fn a_start_that_fails_names_the_program_and_the_working_directory() {
        let dir = stand_in_game("direct-fails");
        let game = dir.path();
        let game_dir = GameDir::new(game.to_path_buf()).expect("a game folder");
        let error = start(&game_dir, &game.join("home"))
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
