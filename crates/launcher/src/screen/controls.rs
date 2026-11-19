//! The controllers of the headset screen: the device profiles the launch app reads beside its own
//! exe, and the log lines of what the controllers do.
//!
//! A press is the change from released to pressed, so a held button logs once. `select` counts as
//! pressed above [`SELECT_PRESS`].

use std::env;
use std::path::Path;
use std::time::Duration;

use device_profile::folder::read_folder;
use device_profile::profile::{DeviceProfile, Hand};
use diag::error_chain;
use diag::rate::RateLimit;
use diag::warn_every;
use tracing::{info, warn};
use xr::error::XrError;
use xr::input::{ButtonState, HandInput, Input};
use xr::session::{FrameTiming, Session};

/// The folder of the device profiles, beside the launch app's exe. `tools/deploy.ps1` stages it.
const PROFILES_FOLDER: &str = "profiles";

/// The `select` amount above which `select` counts as pressed.
const SELECT_PRESS: f32 = 0.7;

/// The shortest time between two warnings of a failed controller read.
const READ_FAILURE_PERIOD: Duration = Duration::from_secs(10);

/// The controllers across the screen's frames: what they held in the frame before, and the rate
/// limit of the warning of a failed read.
pub(crate) struct Controllers {
    watch: ControllerWatch,
    read_failure: RateLimit,
}

impl Controllers {
    pub(crate) fn new() -> Self {
        Self { watch: ControllerWatch::default(), read_failure: RateLimit::new() }
    }

    /// Reads the controllers' input of the frame of `timing` and logs what they did. A failed read
    /// that is no loss of the session warns at most once per [`READ_FAILURE_PERIOD`] and counts as
    /// the idle input, since the screen works without controllers.
    ///
    /// # Errors
    ///
    /// Returns the [`XrError`] of a read that lost the session.
    pub(crate) fn read(&mut self, session: &Session, timing: FrameTiming) -> Result<(), XrError> {
        let (input, failure) = frame_input(session.read_input(timing))?;
        if let Some(error) = failure {
            warn_every!(
                self.read_failure,
                READ_FAILURE_PERIOD,
                error = %error,
                "the controllers could not be read, so they count as idle"
            );
        }
        self.watch.update(&input, log);
        Ok(())
    }
}

/// The input of a frame's controller read, and the failure to warn of when the read failed
/// without losing the session, which gives the idle input.
///
/// # Errors
///
/// Returns the [`XrError`] of a read that lost the session.
fn frame_input(read: Result<Input, XrError>) -> Result<(Input, Option<XrError>), XrError> {
    match read {
        Ok(input) => Ok((input, None)),
        Err(error) if error.is_loss() => Err(error),
        Err(error) => Ok((Input::default(), Some(error))),
    }
}

/// The device profiles in the folder [`PROFILES_FOLDER`] beside the launch app's exe. A missing
/// folder gives no profile, and an unknown exe path or a folder that cannot be listed gives no
/// profile and a warning, so the screen runs without controllers.
pub(crate) fn profiles() -> Vec<DeviceProfile> {
    match env::current_exe() {
        Ok(exe) => read_profiles(&exe.with_file_name(PROFILES_FOLDER)),
        Err(error) => {
            warn!(error = %error, "the launch app's exe path is unknown, so no device profile is read");
            Vec::new()
        }
    }
}

/// The device profiles in `folder`, with one warning per rejected file.
fn read_profiles(folder: &Path) -> Vec<DeviceProfile> {
    let read = match read_folder(folder) {
        Ok(read) => read,
        Err(error) => {
            let error = error_chain::chain(&error);
            warn!(error = %error, "no device profile is read");
            return Vec::new();
        }
    };
    for rejected in &read.rejected {
        let reason = error_chain::chain(&rejected.reason);
        warn!(file = %rejected.file, reason = %reason, "device profile rejected");
    }
    let names: Vec<&str> = read.profiles.iter().map(|profile| profile.name.as_str()).collect();
    info!(folder = %folder.display(), profiles = ?names, "device profiles read");
    read.profiles
}

/// A control whose press the screen logs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Control {
    Menu,
    Select,
    Secondary,
    Middle,
}

impl Control {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Menu => "menu",
            Self::Select => "select",
            Self::Secondary => "secondary",
            Self::Middle => "middle",
        }
    }
}

/// What a hand did since the frame before.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Change {
    /// The hand's `aim` pose became tracked.
    Tracked(Hand),
    /// The hand's `aim` pose stopped being tracked.
    Lost(Hand),
    Pressed(Hand, Control),
}

/// Whether a hand's `aim` pose is tracked.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Tracking {
    #[default]
    Lost,
    Tracked,
}

/// What one hand held in the frame before.
#[derive(Clone, Copy, Debug, Default)]
struct HandState {
    aim: Tracking,
    menu: ButtonState,
    select: ButtonState,
    secondary: ButtonState,
    middle: ButtonState,
}

impl HandState {
    fn of(input: &HandInput) -> Self {
        let tracking = if input.aim.is_some() { Tracking::Tracked } else { Tracking::Lost };
        let select =
            if input.select > SELECT_PRESS { ButtonState::Pressed } else { ButtonState::Released };
        Self {
            aim: tracking,
            menu: input.menu,
            select,
            secondary: input.secondary,
            middle: input.middle,
        }
    }
}

/// The controllers' state of the frame before, which turns each frame's input into changes.
#[derive(Debug, Default)]
struct ControllerWatch {
    left: HandState,
    right: HandState,
}

impl ControllerWatch {
    /// Calls `on_change` with each change from the frame before to `input`.
    fn update(&mut self, input: &Input, mut on_change: impl FnMut(Change)) {
        for hand in Hand::ALL {
            let now = HandState::of(input.hand(hand));
            let before = match hand {
                Hand::Left => &mut self.left,
                Hand::Right => &mut self.right,
            };
            report(hand, *before, now, &mut on_change);
            *before = now;
        }
    }
}

/// Calls `on_change` with each change of `hand` from `before` to `now`.
fn report(hand: Hand, before: HandState, now: HandState, on_change: &mut impl FnMut(Change)) {
    match (before.aim, now.aim) {
        (Tracking::Lost, Tracking::Tracked) => on_change(Change::Tracked(hand)),
        (Tracking::Tracked, Tracking::Lost) => on_change(Change::Lost(hand)),
        (Tracking::Lost, Tracking::Lost) | (Tracking::Tracked, Tracking::Tracked) => {}
    }
    let buttons = [
        (Control::Menu, before.menu, now.menu),
        (Control::Select, before.select, now.select),
        (Control::Secondary, before.secondary, now.secondary),
        (Control::Middle, before.middle, now.middle),
    ];
    let pressed = (ButtonState::Released, ButtonState::Pressed);
    for (control, _, _) in buttons.into_iter().filter(|(_, was, is)| (*was, *is) == pressed) {
        on_change(Change::Pressed(hand, control));
    }
}

/// Logs `change` as one line. A plain `info!` fits the frame loop here, since a change comes from a
/// press or a tracking change and never from each frame.
fn log(change: Change) {
    match change {
        Change::Tracked(hand) => info!(hand = hand.as_str(), "the hand's aim pose is tracked"),
        Change::Lost(hand) => {
            info!(hand = hand.as_str(), "the hand's aim pose is no longer tracked");
        }
        Change::Pressed(hand, control) => {
            info!(hand = hand.as_str(), control = control.as_str(), "controller press");
        }
    }
}

#[cfg(test)]
mod tests;
