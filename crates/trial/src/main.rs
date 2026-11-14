//! The trial runner, a development tool. Runs one trial with no person at the PC.
//!
//! `trial.exe --name <name>` starts the game with the mod, waits for its first frame, measures the
//! frame times from the core's perf summary lines, saves frames as PNG files, closes the game and
//! writes `<out>/<UTC time>-<name>/`. `steps.rs` runs the steps against two traits, and `live.rs`
//! drives the game and the viewer link through the `launch` and `viewer-client` crates. It exits
//! with 0 when every step passed, 1 when a step failed or the result could not be recorded, and 2
//! for a bad command line.

#![forbid(unsafe_code)]

mod args;
mod folder;
mod live;
mod perf;
mod pixels;
mod png_file;
mod steps;
#[cfg(test)]
mod test_dir;

use std::env;
use std::io::{self, Write as _};
use std::process::ExitCode;
use std::time::SystemTime;

use diag::logging::{self, LogError, LogName};
use tracing::{dispatcher, error};

use crate::args::{ArgsError, Options, USAGE};
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
    match run(&options) {
        Ok(code) => code,
        Err(failure) => {
            // A failed write to stderr leaves the exit code as the failure's only record.
            match writeln!(io::stderr(), "trial: {}", steps::cause(&failure)) {
                Ok(()) | Err(_) => ExitCode::from(TRIAL_FAILED),
            }
        }
    }
}

fn report_usage(error: &ArgsError) -> ExitCode {
    // A failed write to stderr leaves the exit code as the failure's only record.
    match writeln!(io::stderr(), "{error}\n{USAGE}") {
        Ok(()) | Err(_) => ExitCode::from(USAGE_FAILED),
    }
}

/// Creates the result folder, logs the trial into it and runs the steps.
fn run(options: &Options) -> Result<ExitCode, Failure> {
    let since_epoch = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default();
    let folder = folder::create(&options.settings.out, &options.name, since_epoch)?;
    let logging = logging::start(&folder, LogName::Trial)?;
    let mut game = LiveGame::new(&options.settings);
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
