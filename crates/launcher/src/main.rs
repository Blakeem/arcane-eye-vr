//! The VR launch app. Writes the launch token and starts the game through Steam.
//!
//! `arcane-eye-vr.exe` creates the launch token, opens the game's Steam URL, then polls the token's
//! status until the game reports the mode's goal status or a failure. Exiting removes the token.
//! The decisions are pure functions in `args.rs` and `decide.rs`. `raw.rs` holds every Windows call.

#![windows_subsystem = "windows"]

mod args;
mod decide;
mod failure;
mod raw;
mod registry;

use std::ffi::OsString;
use std::process::ExitCode;
use std::time::{Instant, SystemTime};
use std::{env, process, thread};

use diag::logging::{self, LogName, Logging};
use ipc_layout::launch_token::{self, LaunchToken, Mode, Status};
use tracing::{dispatcher, error, info, warn};

use crate::decide::Poll;
use crate::failure::Failure;
use crate::raw::TokenMapping;
use crate::registry::{Hive, RegistryValue};

/// The Steam URL that starts Baldur's Gate 3, app id 1086940.
const STEAM_URL: &str = "steam://rungameid/1086940";

/// The active OpenXR runtime's manifest path, which the runtime's installer writes.
const ACTIVE_RUNTIME: RegistryValue = RegistryValue {
    hive: Hive::LocalMachine,
    key: r"SOFTWARE\Khronos\OpenXR\1",
    name: "ActiveRuntime",
};

/// 1 while Steam runs the game. Steam writes it, and Valve does not document it.
const GAME_RUNNING: RegistryValue = RegistryValue {
    hive: Hive::CurrentUser,
    key: r"Software\Valve\Steam\Apps\1086940",
    name: "Running",
};

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
        decide::check_runtime(ACTIVE_RUNTIME.read_string()?)?;
    }
    check_game_not_running()?;
    let token = LaunchToken::new(mode, process::id(), unix_ms_now());
    let mapping = TokenMapping::create(launch_token::MAPPING_NAME, &token)?;
    raw::open_url(STEAM_URL)?;
    info!(url = STEAM_URL, "Steam URL opened");
    wait_for_goal(mode, &mapping)
}

/// Fails when Steam reports the game running. A failed read is a warn line.
fn check_game_not_running() -> Result<(), Failure> {
    let running = GAME_RUNNING.read_dword();
    if let Err(error) = &running {
        warn!(error = %error, "could not read whether the game runs, so the launch goes ahead");
    }
    decide::check_not_running(&running)
}

/// Polls the token's status until the game reports the goal status, a failure or the timeout.
fn wait_for_goal(mode: Mode, mapping: &TokenMapping) -> Result<(), Failure> {
    let started = Instant::now();
    let mut last_status: Option<Status> = None;
    loop {
        let token = LaunchToken::decode(&mapping.read());
        if let Ok(read) = token {
            let status = read.status().ok();
            if status != last_status {
                info!(status = ?status, game_pid = read.game_pid(), "game status changed");
                last_status = status;
            }
        }
        if decide::poll(mode, token, started.elapsed())? == Poll::Done {
            info!("goal status reached, so the launch app exits");
            return Ok(());
        }
        // thread::sleep is banned for game threads. Parking waits the same, and an early wake only
        // polls early.
        thread::park_timeout(decide::POLL_INTERVAL);
    }
}

/// Milliseconds since the Unix epoch. A clock set before 1970 reads as 0, since the token's
/// timestamp is only a record.
fn unix_ms_now() -> u64 {
    let since_epoch = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default();
    u64::try_from(since_epoch.as_millis()).unwrap_or(u64::MAX)
}
