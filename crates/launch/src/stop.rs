//! Stopping the game: a window close that lets it quit as the player would, and an end for a game
//! that did not quit.
//!
//! A caller that holds the game's `Child` keeps its process id bound to the game, since Windows reuses
//! no id while a handle to the process is open.

use std::time::Duration;

use tracing::{info, warn};
use windows::Win32::System::Threading::{PROCESS_SYNCHRONIZE, PROCESS_TERMINATE};

use crate::raw::{self, Process};

/// The longest wait for a process to go once it was ended. Ending returns before the process is gone.
const END_WAIT: Duration = Duration::from_secs(10);

/// Whether a process ran at the end of a wait.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProcessState {
    Exited,
    Running,
}

/// Why a close or an end failed.
#[derive(Debug, thiserror::Error)]
pub enum StopError {
    #[error("could not open process {pid}")]
    Open { pid: u32, source: windows::core::Error },
    #[error("could not wait for process {pid}")]
    Wait { pid: u32, source: windows::core::Error },
    #[error("could not end process {pid}")]
    End { pid: u32, source: windows::core::Error },
    #[error("process {pid} still ran {END_WAIT:?} after it was ended")]
    StillRunning { pid: u32 },
}

/// Asks every top-level window of process `pid` to close, then waits up to `timeout` for the process
/// to exit.
///
/// # Errors
///
/// Returns [`StopError::Open`] when the process cannot be opened and [`StopError::Wait`] when the
/// wait fails.
pub fn close(pid: u32, timeout: Duration) -> Result<ProcessState, StopError> {
    let process = Process::open(pid, PROCESS_SYNCHRONIZE)?;
    let asked = raw::ask_windows_to_close(pid);
    if asked == 0 {
        warn!(
            pid,
            "the process has no top-level window to close, so only the wait can see it exit"
        );
    } else {
        info!(pid, windows = asked, timeout = ?timeout, "asked the process's windows to close");
    }
    process.wait(timeout)
}

/// Ends process `pid` and waits until it is gone.
///
/// An exited process returns `Ok` only while the caller holds a handle to it, such as its `Child`.
/// Without a handle, the open fails, or the id already names another process that this call ends.
///
/// # Errors
///
/// Returns [`StopError::Open`] when the process cannot be opened, [`StopError::Wait`] when a wait
/// fails, [`StopError::End`] when the end call fails on a running process and
/// [`StopError::StillRunning`] when the process outlives the wait after its end.
pub fn end(pid: u32) -> Result<(), StopError> {
    let process = Process::open(pid, PROCESS_SYNCHRONIZE | PROCESS_TERMINATE)?;
    if process.wait(Duration::ZERO)? == ProcessState::Exited {
        return Ok(());
    }
    if let Err(error) = process.terminate() {
        return settle_failed_end(&process, error);
    }
    warn!(pid, "process ended without a close");
    match process.wait(END_WAIT)? {
        ProcessState::Exited => Ok(()),
        ProcessState::Running => Err(StopError::StillRunning { pid }),
    }
}

/// Returns `Ok` when `process` exited, and `error`, the failure of its end call, otherwise.
///
/// A process that exits between the first wait in `end` and the end call fails that call with
/// access denied, and a process that is gone needs no end.
pub(crate) fn settle_failed_end(process: &Process, error: StopError) -> Result<(), StopError> {
    match process.wait(Duration::ZERO)? {
        ProcessState::Exited => Ok(()),
        ProcessState::Running => Err(error),
    }
}
