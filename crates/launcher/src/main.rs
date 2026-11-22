//! The VR launch app. Writes the launch token, starts the game and shows its frames in the headset,
//! or asks the running game to reload its settings.
//!
//! `arcane-eye-vr.exe` creates the launch token, opens the game's Steam URL, then polls the token's
//! status until the game reports the mode's goal status or a failure. The token is removed once the
//! wait ends. In the modes `screen` and `vr`, the app opens the OpenXR session first and runs
//! those steps from the session's frame loop once the session runs, or skips them when a game with
//! the mod already runs. It shows the game's frames in the headset, on a screen or around the
//! player, until the game or the OpenXR session ends. `arcane-eye-vr.exe --reload-settings`
//! instead asks the running game to read the settings files again, and exits. The `launch` crate
//! runs each launch step and the reload request, `start.rs` orders the start's steps, and
//! `screen.rs` runs the headset session. This crate holds the command line, the player's messages
//! and the message box in `raw.rs`.

#![windows_subsystem = "windows"]

mod args;
mod failure;
mod raw;
mod screen;
mod start;

use std::env;
use std::process::ExitCode;

use diag::logging::{self, LogName, Logging};
use diag::root::Root;
use ipc_layout::launch_token::Mode;
use launch::status::{self, Goal};
use launch::{checks, reload};
use tracing::{dispatcher, error, info};

use crate::args::{ArgsError, Command, LaunchMode};
use crate::failure::Failure;
use crate::screen::game::{self, GameChoice};
use crate::screen::{RunFailure, View};
use crate::start::{POLL_INTERVAL, Start, TIMEOUT};

fn main() -> ExitCode {
    let parsed = args::parse(env::args_os().skip(1));
    let (logging, root) = match start_logging(args::log_name(&parsed)) {
        Ok(started) => started,
        Err(failure) => {
            raw::show_error(&failure.to_string());
            return ExitCode::FAILURE;
        }
    };
    let ran = dispatcher::with_default(&logging.dispatch, || {
        let ran = run(parsed, &root);
        if let Err(failure) = &ran {
            error!(error = %failure, "launch app failed");
            raw::show_error(&failure.to_string());
        }
        ran
    });
    // A log write failure has no log left to report to, so the exit code is its only record.
    match (ran, logging.handle.shutdown()) {
        (Ok(()), Ok(())) => ExitCode::SUCCESS,
        (Err(_), _) | (_, Err(_)) => ExitCode::FAILURE,
    }
}

/// Starts the launch app's log `log` in the root folder's log folder, and logs a warning there when
/// the root folder ignored the root folder variable.
fn start_logging(log: LogName) -> Result<(Logging, Root), Failure> {
    let root = Root::find()?;
    let logging = logging::start(&root.log_dir(), log)?;
    dispatcher::with_default(&logging.dispatch, || root.warn_ignored());
    Ok((logging, root))
}

/// Runs the command that the command line `parsed` names.
fn run(parsed: Result<Command, ArgsError>, root: &Root) -> Result<(), Failure> {
    match parsed? {
        Command::Launch(mode) => launch(mode, root),
        Command::ReloadSettings => reload_settings(),
    }
}

/// Runs the launch in `mode`. It returns once the game reports the mode's goal status, or in the
/// modes `screen` and `vr` once the headset session ends.
fn launch(mode: LaunchMode, root: &Root) -> Result<(), Failure> {
    info!(version = env!("CARGO_PKG_VERSION"), mode = mode.as_str(), "launch started");
    match mode {
        LaunchMode::DesktopStereo => start_game(mode.token_mode()),
        LaunchMode::Screen => headset(View::Screen, root),
        LaunchMode::Immersive => headset(View::Immersive, root),
    }
}

/// The launch mode whose headset session shows the game's frames as `view` says.
const fn headset_mode(view: View) -> LaunchMode {
    match view {
        View::Screen => LaunchMode::Screen,
        View::Immersive => LaunchMode::Immersive,
    }
}

/// Checks the runtime, then opens the headset session and, unless a game with the mod runs, starts
/// the game in the token mode of the launch mode of `view` once the session runs. It shows the
/// game's frames in the headset as `view` says until the session ends. A failure's fix names that
/// launch mode.
fn headset(view: View, root: &Root) -> Result<(), Failure> {
    let mode = headset_mode(view);
    let manifest = checks::check_runtime()?;
    let choice = game::find_game()?;
    match choice {
        GameChoice::Running => {
            info!("a game with the mod runs, so the headset shows it and no game starts");
            if view == View::Immersive {
                info!(
                    "the running game's launch mode is unknown, and each of its frames with no \
                     kept head pose shows on the flat screen"
                );
            }
        }
        GameChoice::Start => checks::check_game_not_running()?,
    }
    match screen::run(&manifest, root.path(), view, Start::new(choice, mode.token_mode())) {
        Ok(()) => Ok(()),
        Err(RunFailure::Screen(cause)) => Err(Failure::Headset { cause, mode }),
        Err(RunFailure::Start(error)) => Err(error.into()),
    }
}

/// Starts the game through Steam in `mode` and waits until it reports the mode's goal status.
fn start_game(mode: Mode) -> Result<(), Failure> {
    checks::check_game_not_running()?;
    let mapping = start::begin(mode)?;
    status::wait_for_goal(&mapping, Goal::of(mode), POLL_INTERVAL, TIMEOUT)?;
    Ok(())
}

/// Asks the running game to read the settings files again. It starts no game and creates no token.
fn reload_settings() -> Result<(), Failure> {
    info!(version = env!("CARGO_PKG_VERSION"), "settings reload requested");
    reload::request()?;
    info!("settings reload signaled");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{LaunchMode, View, headset_mode};

    #[test]
    fn each_headset_view_belongs_to_the_launch_mode_that_shows_it() {
        assert_eq!(headset_mode(View::Screen), LaunchMode::Screen);
        assert_eq!(headset_mode(View::Immersive), LaunchMode::Immersive);
    }
}
