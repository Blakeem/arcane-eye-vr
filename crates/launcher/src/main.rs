//! The VR launch app. Writes the launch token, starts the game and shows its frames in the headset.
//!
//! `arcane-eye-vr.exe` creates the launch token, opens the game's Steam URL, then polls the token's
//! status until the game reports the mode's goal status or a failure. The token is removed once the
//! wait ends. In the mode `screen`, the app skips those steps when a game with the mod already
//! runs. It then shows the game's frames on a screen in the headset until the game or the OpenXR
//! session ends. The `launch` crate runs each launch step, and `screen.rs` runs the headset screen.
//! This crate holds the command line, the player's messages and the message box in `raw.rs`.

#![windows_subsystem = "windows"]

mod args;
mod failure;
mod raw;
mod screen;

use std::env;
use std::ffi::OsString;
use std::process::ExitCode;
use std::time::Duration;

use diag::logging::{self, LogName, Logging};
use diag::root::Root;
use ipc_layout::launch_token::Mode;
use launch::status::{self, Goal};
use launch::{checks, steam, token};
use tracing::{dispatcher, error, info};

use crate::args::LaunchMode;
use crate::failure::Failure;
use crate::screen::GameChoice;

/// The time between two reads of the launch token's status.
const POLL_INTERVAL: Duration = Duration::from_millis(250);

/// The longest wait for the goal status. Steam can hold a launch for an update or a cloud sync.
const TIMEOUT: Duration = Duration::from_mins(10);

/// The launch token's mode of a game the headset screen shows.
const SCREEN_GAME_MODE: Mode = Mode::DesktopStereo;

fn main() -> ExitCode {
    let (logging, root) = match start_logging() {
        Ok(started) => started,
        Err(failure) => {
            raw::show_error(&failure.to_string());
            return ExitCode::FAILURE;
        }
    };
    let launched = dispatcher::with_default(&logging.dispatch, || {
        let launched = launch(env::args_os().skip(1), &root);
        if let Err(failure) = &launched {
            error!(error = %failure, "launch failed");
            raw::show_error(&failure.to_string());
        }
        launched
    });
    // A log write failure has no log left to report to, so the exit code is its only record.
    match (launched, logging.handle.shutdown()) {
        (Ok(()), Ok(())) => ExitCode::SUCCESS,
        (Err(_), _) | (_, Err(_)) => ExitCode::FAILURE,
    }
}

/// Starts the launch app's log in the root folder's log folder, and logs a warning there when the root
/// folder ignored the root folder variable.
fn start_logging() -> Result<(Logging, Root), Failure> {
    let root = Root::find()?;
    let logging = logging::start(&root.log_dir(), LogName::Launcher)?;
    dispatcher::with_default(&logging.dispatch, || root.warn_ignored());
    Ok((logging, root))
}

/// Runs the mode that `args` asks for. It returns once the game reports the mode's goal status, or
/// in the mode `screen` once the headset screen ends.
fn launch(args: impl IntoIterator<Item = OsString>, root: &Root) -> Result<(), Failure> {
    let mode = args::parse(args)?;
    info!(version = env!("CARGO_PKG_VERSION"), mode = mode.as_str(), "launch started");
    match mode {
        LaunchMode::Game(mode) => {
            if mode == Mode::Vr {
                checks::check_runtime()?;
            }
            start_game(mode)
        }
        LaunchMode::Screen => {
            let manifest = checks::check_runtime()?;
            match screen::find_game()? {
                GameChoice::Running => {
                    info!("a game with the mod runs, so the screen shows it and no game starts");
                }
                GameChoice::Start => start_game(SCREEN_GAME_MODE)?,
            }
            Ok(screen::run(&manifest, screen::settings(root.path()))?)
        }
    }
}

/// Starts the game through Steam in `mode` and waits until it reports the mode's goal status.
fn start_game(mode: Mode) -> Result<(), Failure> {
    checks::check_game_not_running()?;
    let mapping = token::create(mode)?;
    steam::start()?;
    status::wait_for_goal(&mapping, Goal::of(mode), POLL_INTERVAL, TIMEOUT)?;
    Ok(())
}
