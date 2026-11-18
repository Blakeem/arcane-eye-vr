//! The headset-screen step and the screen-exit step, against the launch app and SteamVR's headset
//! window behind the `Headset` trait, so a test drives a stand-in.

use std::path::Path;
use std::process::ExitStatus;
use std::time::Duration;

use tracing::{info, warn};

use super::{Frames, Game, StepError, cause};
use crate::compare::Limit;
use crate::folder;
use crate::headset::{self, HeadsetError};
use crate::home::{self, HOME_FOLDER};
use crate::pixels::{self, RgbImage};
use crate::png_file;

/// The name of the launch app's standard output and standard error in the result folder.
const SCREEN_APP_FILE: &str = "screen-app.txt";

/// The name of the headset window's copy in the result folder.
const HEADSET_PNG: &str = "headset.png";

/// The name of the comparison of the eyes' pictures with the frame in the result folder.
const HEADSET_TXT: &str = "headset.txt";

/// The time between two looks for the headset window and its screen.
const LOOK_INTERVAL: Duration = Duration::from_secs(1);

/// The longest wait for the headset window to show a screen. The measured screen needed 6 s or less
/// from the launch app's start.
const SCREEN_TIMEOUT: Duration = Duration::from_secs(30);

/// The longest wait after the close for the launch app to exit.
pub(super) const EXIT_TIMEOUT: Duration = Duration::from_secs(30);

/// The launch app's screen mode and SteamVR's headset window as the headset steps drive them.
pub(crate) trait Headset {
    /// Checks that the launch app exists beside this program.
    fn check(&mut self) -> Result<(), StepError>;
    /// Starts the launch app's screen mode with `home`, an absolute path, as its root folder, and
    /// with its standard output and standard error written to `output`.
    fn start(&mut self, home: &Path, output: &Path) -> Result<(), StepError>;
    /// Whether the start passed, so a launch app process exists.
    fn started(&self) -> bool;
    /// Makes the headset window topmost without activating it and copies it from the screen, or
    /// returns `None` when no headset window exists.
    fn copy_window(&mut self) -> Result<Option<RgbImage>, StepError>;
    /// Waits up to `timeout` for the launch app to exit and returns its exit status, or `None` when
    /// it still runs.
    fn wait_exit(&mut self, timeout: Duration) -> Result<Option<ExitStatus>, StepError>;
    /// Ends the launch app and waits until it is gone.
    fn end(&mut self) -> Result<(), StepError>;
}

/// Starts the launch app with the game's root folder in the result folder `folder`, so it reads the
/// trial's settings and writes its log there. Then waits for the headset window to show a screen,
/// reads a frame, copies the headset window right after it and writes `headset.png` and
/// `headset.txt`. Fails when an eye's picture is not closer to its own half of the frame, or
/// differs from it by more than `limit`.
pub(super) fn screen_step(
    game: &mut impl Game,
    frames: &mut impl Frames,
    headset: &mut impl Headset,
    limit: Option<Limit>,
    folder: &Path,
) -> Result<(), StepError> {
    let home = home::home_path(folder, HOME_FOLDER)?;
    let output = folder.join(SCREEN_APP_FILE);
    headset.start(&home, &output)?;
    frames.start_heartbeat()?;
    wait_for_screen(game, headset, &output)?;
    let frame = frames.read()?;
    let window = headset.copy_window()?.ok_or(HeadsetError::WindowGone)?;
    png_file::write(&folder.join(HEADSET_PNG), &window)?;
    let comparison = headset::compare(&window, &pixels::to_rgb(&frame)?)?;
    folder::write(folder, HEADSET_TXT, &headset::text(&comparison))?;
    Ok(headset::check(&comparison, limit)?)
}

/// Looks for the headset window and a screen in each of its halves once per second until both
/// show, for at most 30 s. A launch app that exited ends the wait with the path of its `output`,
/// since no screen follows its exit. Otherwise the last look names the failure.
fn wait_for_screen(
    game: &mut impl Game,
    headset: &mut impl Headset,
    output: &Path,
) -> Result<(), StepError> {
    let mut waited = Duration::ZERO;
    loop {
        let copy = headset.copy_window()?;
        let shown = match &copy {
            Some(window) => headset::has_screen(window)?,
            None => false,
        };
        if shown {
            info!(?waited, "the headset window shows a screen");
            return Ok(());
        }
        if let Some(status) = headset.wait_exit(Duration::ZERO)? {
            return Err(HeadsetError::AppExited { status, output: output.to_path_buf() }.into());
        }
        if waited >= SCREEN_TIMEOUT {
            let failure = match copy {
                Some(_) => HeadsetError::NoScreen { waited },
                None => HeadsetError::NoWindow { waited },
            };
            return Err(failure.into());
        }
        game.pause(LOOK_INTERVAL);
        waited = waited.saturating_add(LOOK_INTERVAL);
    }
}

/// Waits up to 30 s for the launch app to exit, and passes on exit code 0. A launch app that still
/// runs, or whose state cannot be read, is ended, so it never outlives the trial.
pub(super) fn exit_step(headset: &mut impl Headset) -> Result<(), StepError> {
    let failure = match headset.wait_exit(EXIT_TIMEOUT) {
        Ok(Some(status)) if status.success() => return Ok(()),
        Ok(Some(status)) => return Err(HeadsetError::AppFailed { status }.into()),
        Ok(None) => StepError::from(HeadsetError::AppRunning { waited: EXIT_TIMEOUT }),
        Err(error) => error,
    };
    // A launch app that may still run outranks the reason it was ended, so the end's failure leads.
    if let Err(end_error) = headset.end() {
        warn!(error = %cause(&failure), "the launch app could not be ended after this failure");
        return Err(end_error);
    }
    Err(failure)
}
