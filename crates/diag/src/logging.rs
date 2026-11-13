//! Session logging: starts the log ring, the ring layer's subscriber and the log writer thread.
//!
//! [`start`] builds the subscriber but never installs it, so the caller decides whether it becomes the
//! process-wide default or only a scoped one, as in a test.

use std::fs::{self, File};
use std::io;
use std::os::windows::fs::OpenOptionsExt as _;
use std::path::{Path, PathBuf};
use std::process;
use std::sync::Arc;
use std::sync::mpsc::{self, Sender};
use std::thread::JoinHandle;

use tracing::{Dispatch, dispatcher, warn};
use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::layer::SubscriberExt as _;
use windows::Win32::Storage::FileSystem::{FILE_SHARE_READ, FILE_SHARE_WRITE};

use crate::layer::{Base, RingLayer};
use crate::raw;
use crate::ring::LogRing;
use crate::rotation::{self, PruneError, RotateError};
use crate::writer::{self, Clock};

/// Why session logging failed to start or to write.
#[derive(Debug, thiserror::Error)]
pub enum LogError {
    #[error("could not resolve the local app data folder")]
    LocalAppData(#[from] windows::core::Error),
    #[error("could not create the log folder {path}")]
    CreateDir { path: PathBuf, source: io::Error },
    #[error("could not create the session log {path}")]
    Create { path: PathBuf, source: io::Error },
    #[error("could not start the log writer thread")]
    Spawn(#[source] io::Error),
    #[error("could not write the session log")]
    Write(#[from] io::Error),
    #[error("the log writer thread panicked")]
    WriterPanicked,
}

/// Running session logging: the subscriber to install and the handle that keeps the writer running.
pub struct Logging {
    pub handle: LogHandle,
    pub dispatch: Dispatch,
}

/// The running log writer thread. Dropping the handle lets the thread drain the ring once more and end.
pub struct LogHandle {
    stop: Sender<()>,
    thread: JoinHandle<Result<(), LogError>>,
    file_path: PathBuf,
}

impl LogHandle {
    /// The current session log file.
    pub fn file_path(&self) -> &Path {
        &self.file_path
    }

    /// Stops the log writer thread after it writes every finished line, and waits for it.
    ///
    /// # Errors
    ///
    /// Returns the write error that ended the thread, or [`LogError::WriterPanicked`].
    pub fn shutdown(self) -> Result<(), LogError> {
        drop(self.stop);
        match self.thread.join() {
            Ok(result) => result,
            Err(_payload) => Err(LogError::WriterPanicked),
        }
    }
}

/// Which program a log belongs to. Each one rotates its own files in the shared log folder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogName {
    /// The core DLL's session log, `arcane-eye-vr.log`.
    Core,
    /// The launch app's log, `arcane-eye-vr-launcher.log`.
    Launcher,
}

impl LogName {
    /// The file name before the rotation index and the extension.
    pub(crate) const fn stem(self) -> &'static str {
        match self {
            Self::Core => "arcane-eye-vr",
            Self::Launcher => "arcane-eye-vr-launcher",
        }
    }
}

/// The session log folder, `%LOCALAPPDATA%\arcane-eye-vr\logs`.
///
/// # Errors
///
/// Returns [`LogError::LocalAppData`] when Windows cannot resolve the folder.
pub fn session_log_dir() -> Result<PathBuf, LogError> {
    Ok(raw::local_app_data()?.join("arcane-eye-vr").join("logs"))
}

/// Rotates the files of `log` in `log_dir`, opens a new session log and starts the log writer thread.
///
/// A failed rotation is not an error. The session then logs to a file named with the process id, and
/// its first line names the rename that failed. Each start removes all but the 4 newest earlier fallback
/// files of `log`, and a failed removal is a warn line, not an error.
///
/// # Errors
///
/// Returns the step that failed: the folder, the new file or the thread start.
pub fn start(log_dir: &Path, log: LogName) -> Result<Logging, LogError> {
    let session = open_session(log_dir, log)?;
    let ring = Arc::new(LogRing::new());
    let (stop, stop_receiver) = mpsc::channel();
    let thread = writer::spawn(Arc::clone(&ring), session.file, Clock::now(), stop_receiver)?;
    let dispatch = Dispatch::new(Base.with(LevelFilter::INFO).with(RingLayer::new(ring)));
    dispatcher::with_default(&dispatch, || warn_start_failures(&session.failures));
    Ok(Logging { handle: LogHandle { stop, thread, file_path: session.path }, dispatch })
}

/// Logs the start-up failures that did not stop the session as the first lines of its log.
fn warn_start_failures(failures: &StartFailures) {
    if let Some(error) = &failures.rotate {
        warn!(
            from = %error.from.display(),
            to = %error.to.display(),
            error = %error.source,
            "session log rotation failed, so this session logs to its own file"
        );
    }
    if let Some(error) = &failures.prune {
        warn!(
            path = %error.path.display(),
            error = %error.source,
            "fallback session log prune failed, so older fallback logs stay"
        );
    }
}

/// A new session log file and its start-up failures.
struct SessionFile {
    path: PathBuf,
    file: File,
    failures: StartFailures,
}

/// The start-up steps that failed without stopping the session.
struct StartFailures {
    rotate: Option<RotateError>,
    prune: Option<PruneError>,
}

fn open_session(log_dir: &Path, log: LogName) -> Result<SessionFile, LogError> {
    if let Err(source) = fs::create_dir_all(log_dir) {
        return Err(LogError::CreateDir { path: log_dir.to_path_buf(), source });
    }
    let prune = rotation::prune_fallbacks(log_dir, log).err();
    let (path, rotate) = match rotation::rotate(log_dir, log) {
        Ok(()) => (rotation::session_path(log_dir, log, 0), None),
        Err(error) => (rotation::fallback_path(log_dir, log, process::id()), Some(error)),
    };
    // Append, so a reused process id never truncates an older session's fallback log. No delete share,
    // so another process's rotation cannot rename or replace this live log and takes its fallback.
    let share = FILE_SHARE_READ.0 | FILE_SHARE_WRITE.0;
    match File::options().create(true).append(true).share_mode(share).open(&path) {
        Ok(file) => Ok(SessionFile { path, file, failures: StartFailures { rotate, prune } }),
        Err(source) => Err(LogError::Create { path, source }),
    }
}
