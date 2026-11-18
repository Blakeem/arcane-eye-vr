//! The background trial: its game runs on a Windows desktop of the trial's own, so the game window
//! takes neither the keyboard focus nor the mouse of the desktop the user works on.
//!
//! `trial.exe --background` creates the desktop, runs a copy of itself there with the same command
//! line, and exits with the copy's exit code. The copy sees its start on a trial desktop and runs
//! the steps. The game it starts inherits the desktop, so the copy's posted key and close reach
//! the game's windows as in a trial without the option. A measured background trial passed every
//! step there. The game renders 30 frames per second on a desktop that is not the input desktop,
//! and Windows moves the cursor of the input desktop only, so the check step refuses a perf window
//! and a cursor position.

use std::io;
use std::{env, process};

use launch::desktop::{self, Desktop, DesktopError};

use crate::args::{Background, Settings};

/// The start of the name of every desktop a background trial creates.
const DESKTOP_PREFIX: &str = "arcane-eye-trial-";

/// What this process does with a trial that asks for its own desktop.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Role {
    /// This process started on the trial desktop with this name, so it runs the steps.
    RunSteps(String),
    /// This process creates a desktop and runs a copy of itself there.
    StartCopy,
}

/// Why the check step refuses a background trial.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub(crate) enum Refused {
    #[error(
        "--background runs the game on a desktop that is not the input desktop, where it renders \
         30 frames per second, so a perf window measures nothing useful. Pass --perf-lines 0"
    )]
    PerfWindow,
    #[error(
        "--background runs the game on a desktop that is not the input desktop, where it renders \
         30 frames per second and where --cursor cannot move the cursor. Drop --cursor or \
         --background"
    )]
    Cursor,
}

/// Why the copy of this process on its own desktop did not run to its end.
#[derive(Debug, thiserror::Error)]
pub(crate) enum CopyError {
    #[error("could not find this program's path")]
    Program(#[source] io::Error),
    #[error(transparent)]
    Desktop(#[from] DesktopError),
    #[error("the trial's copy on the desktop {desktop} ended with the exit code {code:#x}")]
    Exit { desktop: String, code: u32 },
}

/// The role of a process whose start named the desktop `startup_desktop`.
pub(crate) fn role(startup_desktop: Option<&str>) -> Role {
    match startup_desktop {
        Some(name) if is_trial_desktop(name) => Role::RunSteps(name.to_owned()),
        Some(_) | None => Role::StartCopy,
    }
}

/// Whether `name` is the name [`desktop_name`] gives.
fn is_trial_desktop(name: &str) -> bool {
    name.strip_prefix(DESKTOP_PREFIX)
        .is_some_and(|pid| !pid.is_empty() && pid.bytes().all(|byte| byte.is_ascii_digit()))
}

/// The name of the desktop that the trial process `pid` creates.
pub(crate) fn desktop_name(pid: u32) -> String {
    format!("{DESKTOP_PREFIX}{pid}")
}

/// Fails the check step of a background trial with a perf window or a cursor position.
pub(crate) fn refuse(settings: &Settings) -> Result<(), Refused> {
    if settings.background == Background::Off {
        return Ok(());
    }
    if settings.perf_lines > 0 {
        return Err(Refused::PerfWindow);
    }
    if settings.cursor.is_some() {
        return Err(Refused::Cursor);
    }
    Ok(())
}

/// Creates this process's desktop, runs a copy of this process there with the same command line
/// and returns the copy's exit code. The desktop closes when this function returns, on every path.
/// A game the copy left running ends then too, and so does every process on the desktop when this
/// process ends early, such as on a Ctrl+C.
pub(crate) fn run_copy() -> Result<u8, CopyError> {
    let program = match env::current_exe() {
        Ok(program) => program,
        Err(source) => return Err(CopyError::Program(source)),
    };
    let command_line = desktop::own_command_line();
    let own = Desktop::create(&desktop_name(process::id()))?;
    let code = own.run(&program, &command_line)?;
    exit_code(code).ok_or_else(|| CopyError::Exit { desktop: own.name().to_owned(), code })
}

/// The copy's exit code as this process's exit code, or `None` when it is no trial exit code, such
/// as the code of a process Windows ended at its start.
fn exit_code(code: u32) -> Option<u8> {
    u8::try_from(code).ok()
}

#[cfg(test)]
mod tests {
    use launch::cursor::ClientPoint;

    use super::{Refused, Role, desktop_name, exit_code, refuse, role};
    use crate::args::{Background, Settings};

    #[test]
    fn a_process_started_on_a_trial_desktop_runs_the_steps_and_any_other_starts_a_copy() {
        let name = desktop_name(4242);
        assert_eq!(name, "arcane-eye-trial-4242");
        assert_eq!(role(Some(&name)), Role::RunSteps(name.clone()));
        for other in [None, Some("Default"), Some("arcane-eye-trial-"), Some("arcane-eye-trial-4x")]
        {
            assert_eq!(role(other), Role::StartCopy, "{other:?}");
        }
    }

    #[test]
    fn a_background_trial_with_a_perf_window_or_a_cursor_is_refused() {
        let background =
            Settings { background: Background::On, perf_lines: 0, ..Settings::default() };
        assert_eq!(refuse(&background), Ok(()));
        let perf = Settings { perf_lines: 1, ..background.clone() };
        assert_eq!(refuse(&perf), Err(Refused::PerfWindow));
        let cursor = Settings { cursor: Some(ClientPoint { x: 1, y: 1 }), ..background };
        assert_eq!(refuse(&cursor), Err(Refused::Cursor));
        let input = Settings { cursor: Some(ClientPoint { x: 1, y: 1 }), ..Settings::default() };
        assert_eq!(refuse(&input), Ok(()), "a trial on the input desktop");
        for refused in [Refused::PerfWindow, Refused::Cursor] {
            assert!(refused.to_string().contains("30 frames per second"), "{refused}");
        }
    }

    /// `STATUS_DLL_INIT_FAILED`, the measured exit code of a process started on a desktop that
    /// does not exist.
    const DLL_INIT_FAILED: u32 = 0xC000_0142;

    #[test]
    fn a_trial_exit_code_passes_through_and_a_windows_status_does_not() {
        assert_eq!(exit_code(0), Some(0));
        assert_eq!(exit_code(2), Some(2));
        assert_eq!(exit_code(DLL_INIT_FAILED), None);
    }
}
