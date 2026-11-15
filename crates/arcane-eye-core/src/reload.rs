//! The live reload of the settings files: the published player settings and the poll thread that
//! replaces them.
//!
//! Once per [`POLL_PERIOD`] the thread checks the player file and the dev file through the watch. A
//! changed player file that parses becomes the published settings, which the Present call-site
//! handler copies once per frame. A rejected player file keeps the settings before. The dev flags
//! pick the hooks at start, so a changed dev file only logs that its flags wait for the next start.
//! The reads, the parsing and the logging all run on the poll thread, never on a game thread.

use std::io;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use arc_swap::ArcSwap;
use diag::error_chain;
use settings::dev::DevFlags;
use settings::file::SETTINGS_FILE;
use settings::player::Settings;
use settings::watch::{Changes, Watch};
use tracing::{info, warn};

/// The time between two checks of the files. A player edits the files outside the game, so a delay
/// of this length after a save goes unnoticed, and a check costs two metadata reads.
const POLL_PERIOD: Duration = Duration::from_secs(1);

const THREAD_NAME: &str = "arcane-eye-settings";

/// The player settings in effect, which the poll thread replaces and hook handlers copy.
#[derive(Debug)]
pub(crate) struct Published(ArcSwap<Settings>);

impl Published {
    pub(crate) fn new(settings: Settings) -> Self {
        Self(ArcSwap::from_pointee(settings))
    }

    /// A copy of the settings in effect. It takes no lock and makes no heap allocation.
    pub(crate) fn get(&self) -> Settings {
        **self.0.load()
    }

    fn set(&self, settings: Settings) {
        self.0.store(Arc::new(settings));
    }
}

/// The running poll thread. Dropping it ends the thread at its next wake.
#[derive(Debug)]
pub(crate) struct Reload {
    /// The thread ends once this sender drops.
    _stop: Sender<()>,
    _thread: JoinHandle<()>,
}

/// Starts the poll thread for the files of `watch`, which the core read once at start. `started` are
/// the dev flags of that read.
///
/// # Errors
///
/// Returns the error of a thread that could not start.
pub(crate) fn start(
    watch: Watch,
    started: DevFlags,
    published: Arc<Published>,
) -> io::Result<Reload> {
    start_with(watch, started, published, POLL_PERIOD)
}

fn start_with(
    mut watch: Watch,
    started: DevFlags,
    published: Arc<Published>,
    period: Duration,
) -> io::Result<Reload> {
    let (stop, stopped) = mpsc::channel();
    let builder = thread::Builder::new().name(THREAD_NAME.to_owned());
    let thread = builder.spawn(move || {
        while waits(&stopped, period) {
            step(&mut watch, started, &published);
        }
    })?;
    Ok(Reload { _stop: stop, _thread: thread })
}

/// Waits `period`, and returns whether the stop sender still lives then.
fn waits(stopped: &Receiver<()>, period: Duration) -> bool {
    stopped.recv_timeout(period) == Err(RecvTimeoutError::Timeout)
}

/// Checks the files of `watch` once and applies what changed.
fn step(watch: &mut Watch, started: DevFlags, published: &Published) {
    let Changes { settings, dev } = watch.check();
    match settings {
        Some(Ok(parsed)) => {
            for key in &parsed.unknown_keys {
                warn!(file = SETTINGS_FILE, key = %key, "unknown settings key ignored");
            }
            published.set(parsed.settings);
            info!(ipd_mm = parsed.settings.stereo.ipd_mm.millimeters(), "settings reloaded");
        }
        Some(Err(error)) => warn!(
            error = %error_chain::chain(&error),
            "changed settings file gave no values, so the settings before stay"
        ),
        None => {}
    }
    match dev {
        Some(Ok(flags)) if flags == started => {}
        Some(Ok(flags)) => info!(
            main_view_eye = flags.stereo.main_view_eye.as_str(),
            second_view = flags.stereo.second_view.as_bool(),
            "dev file changed, and its flags apply at the next start"
        ),
        Some(Err(error)) => warn!(
            error = %error_chain::chain(&error),
            "changed dev file gave no values, and the flags read at start stay"
        ),
        None => {}
    }
}

#[cfg(test)]
mod tests;
