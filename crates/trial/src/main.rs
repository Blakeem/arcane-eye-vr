//! The trial runner, a development tool. Runs one trial with no person at the PC.
//!
//! `trial.exe --name <name>` starts the game with the mod, waits for its first frame, measures the
//! frame times from the core's perf summary lines, records the cameras from its camera lines, saves
//! frames as PNG files, saves a burst of published frames of one region when asked, closes the
//! game and writes `<out>/<UTC time>-<name>/`. The game's root folder is `home` in that folder.
//! With `--scene level`, the trial's session posts a key on the title screen and waits until the
//! latest save's level runs. A priming session first reaches the main menu and closes when the game
//! would refuse that load (`level.rs`). With `--background`, it runs a copy of itself and the game
//! on a Windows desktop of its own (`background.rs`). `compare.rs` computes the half difference of
//! each captured frame and the step of each burst frame.
//! `steps.rs` runs the steps against two traits, and `live.rs` drives the game and the viewer link
//! through the `launch` and `viewer-client` crates. It exits with 0 when every step passed, 1 when a
//! step failed or the result could not be recorded, and 2 for a bad command line.

#![forbid(unsafe_code)]

mod args;
mod background;
mod burst;
mod compare;
mod folder;
mod home;
mod level;
mod live;
mod log_tail;
mod mod_check;
mod perf;
mod pixels;
mod place;
mod png_file;
mod report;
mod steps;

use std::env;
use std::error::Error;
use std::io::{self, Write as _};
use std::process::ExitCode;
use std::time::SystemTime;

use diag::logging::{self, LogError, LogName};
use launch::desktop;
use tracing::{dispatcher, error};

use crate::args::{ArgsError, Background, Options, USAGE};
use crate::background::Role;
use crate::folder::FolderError;
use crate::live::{LiveFrames, LiveGame};
use crate::steps::{TRIAL_FAILED, TrialError};

/// The exit code of a bad command line.
const USAGE_FAILED: u8 = 2;

/// Why the trial did not run, or its result was not recorded.
#[derive(Debug, thiserror::Error)]
enum Failure {
    #[error(transparent)]
    Folder(#[from] FolderError),
    #[error("could not write the trial log")]
    Log(#[from] LogError),
    #[error(transparent)]
    Trial(#[from] TrialError),
}

fn main() -> ExitCode {
    let options = match args::parse(env::args_os().skip(1)) {
        Ok(options) => options,
        Err(error) => return report_usage(&error),
    };
    let desktop = match options.settings.background {
        Background::Off => None,
        Background::On => match background::role(desktop::startup_desktop().as_deref()) {
            Role::RunSteps(name) => Some(name),
            Role::StartCopy => return run_copy(),
        },
    };
    match run(&options, desktop) {
        Ok(code) => code,
        Err(failure) => report_failure(&failure),
    }
}

fn report_usage(error: &ArgsError) -> ExitCode {
    // A failed write to stderr leaves the exit code as the failure's only record.
    match writeln!(io::stderr(), "{error}\n{USAGE}") {
        Ok(()) | Err(_) => ExitCode::from(USAGE_FAILED),
    }
}

fn report_failure(failure: &dyn Error) -> ExitCode {
    // A failed write to stderr leaves the exit code as the failure's only record.
    match writeln!(io::stderr(), "trial: {}", steps::cause(failure)) {
        Ok(()) | Err(_) => ExitCode::from(TRIAL_FAILED),
    }
}

/// Runs a copy of this trial on a desktop of its own and exits with the copy's exit code.
fn run_copy() -> ExitCode {
    match background::run_copy() {
        Ok(code) => ExitCode::from(code),
        Err(failure) => report_failure(&failure),
    }
}

/// Creates the result folder, logs the trial into it and runs the steps with the game on
/// `desktop`, the trial desktop this process started on, or `None`.
fn run(options: &Options, desktop: Option<String>) -> Result<ExitCode, Failure> {
    let since_epoch = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default();
    let folder = folder::create(&options.settings.out, &options.name, since_epoch)?;
    let logging = logging::start(&folder, LogName::Trial)?;
    let mut game = LiveGame::new(&options.settings, desktop);
    let mut frames = LiveFrames::new();
    let report = dispatcher::with_default(&logging.dispatch, || {
        let report =
            steps::trial(options, &folder, &mut game, &mut frames, &mut io::stdout().lock());
        if let Err(error) = &report {
            error!(error = %steps::cause(error), "the trial result was not recorded");
        }
        report
    });
    // The trial error says which result was not recorded, so it outranks a failed log shutdown.
    match (report, logging.handle.shutdown()) {
        (Ok(report), Ok(())) => Ok(report.exit_code()),
        (Err(error), _) => Err(error.into()),
        (Ok(_), Err(error)) => Err(error.into()),
    }
}

/// A temp folder for test `name` of this crate.
#[cfg(test)]
fn test_dir(name: &str) -> test_support::TestDir {
    test_support::TestDir::in_temp("arcane-eye-trial", name).expect("create the test folder")
}
