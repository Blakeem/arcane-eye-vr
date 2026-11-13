//! The VR launch app. Writes the launch token and starts the game through Steam.
//!
//! `arcane-eye-vr.exe` creates the launch token, opens the game's Steam URL, then polls the token's
//! status until the game reports the mode's goal status or a failure. Exiting removes the token.
//! The `launch` crate runs each step. This crate holds the command line, the player's messages and
//! the message box in `raw.rs`.

#![windows_subsystem = "windows"]

mod args;
mod failure;
mod raw;

use std::env;
use std::ffi::OsString;
use std::process::ExitCode;
use std::time::Duration;

use diag::logging::{self, LogName, Logging};
use ipc_layout::launch_token::Mode;
use launch::status::{self, Goal};
use launch::{checks, steam, token};
use tracing::{dispatcher, error, info};

use crate::failure::Failure;

/// The time between two reads of the launch token's status.
const POLL_INTERVAL: Duration = Duration::from_millis(250);

/// The longest wait for the goal status. Steam can hold a launch for an update or a cloud sync.
const TIMEOUT: Duration = Duration::from_mins(10);

fn main() -> ExitCode {
    let logging = match start_logging() {
        Ok(logging) => logging,
        Err(failure) => {
            raw::show_error(&failure.to_string());
            return ExitCode::FAILURE;
        }
    };
    let launched = dispatcher::with_default(&logging.dispatch, || {
        let launched = launch(env::args_os().skip(1));
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

fn start_logging() -> Result<Logging, Failure> {
    let log_dir = logging::session_log_dir()?;
    Ok(logging::start(&log_dir, LogName::Launcher)?)
}

/// Runs the launch steps in order and returns once the game reports the goal status of the mode
/// that `args` asks for.
fn launch(args: impl IntoIterator<Item = OsString>) -> Result<(), Failure> {
    let mode = args::parse(args)?;
    info!(version = env!("CARGO_PKG_VERSION"), mode = mode.as_str(), "launch started");
    if mode == Mode::Vr {
        checks::check_runtime()?;
    }
    checks::check_game_not_running()?;
    let mapping = token::create(mode)?;
    steam::start()?;
    status::wait_for_goal(&mapping, Goal::of(mode), POLL_INTERVAL, TIMEOUT)?;
    Ok(())
}
