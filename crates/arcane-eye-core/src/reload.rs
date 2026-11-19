//! The settings reload on request: the published player settings and the thread that replaces them.
//!
//! The thread blocks on the named reload event that the launch app's `--reload-settings` signals,
//! and touches no file between two requests. Each request reads and parses both files once. A player
//! file that parses becomes the published settings, which the Present call-site handler copies once
//! per frame. A rejected player file keeps the settings in use. The dev flags pick the hooks at start,
//! so dev flags that differ from the flags read at start only log that they wait for the next start.
//! The reads, the parsing and the logging all run on this thread, never on a game thread.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread::{self, JoinHandle};

use arc_swap::ArcSwap;
use diag::error_chain;
use ipc_layout::launch_token::Mode;
use settings::dev::DevFlags;
use settings::file;
use settings::player::Settings;
use tracing::{info, warn};

use crate::config::{self, Config};
use crate::raw::reload_event::{self, Event, EventError, Signaled};

const THREAD_NAME: &str = "arcane-eye-settings";

/// The player settings in effect, which the settings thread replaces and hook handlers copy.
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

/// The running settings thread. Dropping it signals the stop event, and the thread ends.
#[derive(Debug)]
pub(crate) struct Reload {
    _stop: Stop,
    _thread: JoinHandle<()>,
}

/// The settings thread's stop event, signaled on drop.
#[derive(Debug)]
struct Stop(Arc<Event>);

impl Drop for Stop {
    fn drop(&mut self) {
        if let Err(error) = self.0.set() {
            warn!(error = %error, "settings thread stop failed, so it waits until the process ends");
        }
    }
}

/// Why the settings thread did not start.
#[derive(Debug, thiserror::Error)]
pub(crate) enum StartError {
    #[error("the settings thread's events were not created")]
    Event(#[from] EventError),
    #[error("the settings thread could not start")]
    Thread(#[from] io::Error),
}

/// Creates the reload event `event_name` and starts the settings thread for the files in `folder`,
/// which the core read once at start. `started` is the config of that read.
///
/// # Errors
///
/// Returns [`StartError::Event`] when the reload event already exists or an event cannot be created,
/// and [`StartError::Thread`] when the thread cannot start.
pub(crate) fn start(
    folder: PathBuf,
    event_name: &str,
    started: Config,
    published: Arc<Published>,
) -> Result<Reload, StartError> {
    let reload = Event::named(event_name)?;
    let stop = Arc::new(Event::unnamed()?);
    let stopped = Arc::clone(&stop);
    let builder = thread::Builder::new().name(THREAD_NAME.to_owned());
    let thread = builder.spawn(move || run(&stopped, &reload, &folder, started, &published))?;
    Ok(Reload { _stop: Stop(stop), _thread: thread })
}

/// Applies the files in `folder` once per signal of `reload`, until `stop` is signaled or a wait fails.
fn run(stop: &Event, reload: &Event, folder: &Path, started: Config, published: &Published) {
    loop {
        match reload_event::wait(stop, reload) {
            Ok(Signaled::Reload) => apply(folder, started, published),
            Ok(Signaled::Stop) => return,
            Err(error) => {
                warn!(error = %error, "settings thread wait failed, so no later request reloads");
                return;
            }
        }
    }
}

/// Reads both files in `folder` once and applies them against `started`, the config read at start.
fn apply(folder: &Path, started: Config, published: &Published) {
    match file::read_settings(folder) {
        Ok(parsed) => {
            config::warn_unknown_keys(&parsed);
            published.set(parsed.settings);
            let stereo = parsed.settings.stereo;
            info!(
                ipd_mm = stereo.ipd_mm.millimeters(),
                depth_offset_percent = stereo.depth_offset_percent.percent(),
                "settings reloaded"
            );
        }
        Err(error) => warn!(
            error = %error_chain::chain(&error),
            "settings file gave no values, so the settings in use stay"
        ),
    }
    match file::read_dev(folder) {
        Ok(flags) => info_changed_dev(started, flags),
        Err(error) => warn!(
            error = %error_chain::chain(&error),
            "dev file gave no values, and the flags read at start stay"
        ),
    }
}

/// Logs the dev file's `flags` that differ from the flags read at start in `started`: one line for the
/// `[stereo]`, `[ui]` and `[viewer]` tables and one for the `[vr]` table. The mode `desktop-stereo`
/// never reads the `[vr]` table, so it logs no `[vr]` change.
fn info_changed_dev(started: Config, flags: DevFlags) {
    let with_started_vr = DevFlags { vr: started.dev.vr, ..flags };
    if with_started_vr != started.dev {
        info!(
            main_view_eye = flags.stereo.main_view_eye.as_str(),
            second_view = flags.stereo.second_view.as_bool(),
            ui_forward_wait_ms = flags.ui.forward_wait_ms.millis(),
            viewer_source = flags.viewer.source.as_str(),
            "dev file changed, and its flags apply at the next start"
        );
    }
    if started.mode == Mode::Vr && flags.vr != started.dev.vr {
        info!(
            horizon = flags.vr.horizon.as_str(),
            test_head = flags.vr.test_head.as_str(),
            test_yaw_deg = flags.vr.test_yaw_deg.degrees(),
            "dev file changed, and its vr flags apply at the next start"
        );
    }
}

#[cfg(test)]
mod tests;
