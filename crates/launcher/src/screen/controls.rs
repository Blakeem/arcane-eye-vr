//! The controllers of the headset screen: the device profiles the launch app reads beside its own
//! exe, the log lines of what the controllers do, and each frame's input with each hand's buttons
//! in the frame before and in that frame.
//!
//! A press is the change from released to pressed, so a held button logs once. The log counts
//! `select` as pressed above [`SELECT_PRESS`]. The menu and the game pointer count `select` as
//! pressed from the frame it rises to [`SELECT_PRESS`] or above until the frame it falls to
//! [`SELECT_RELEASE`] or below.

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

/// The `select` amount above which the log counts `select` as pressed, and at or above which the
/// menu and the game pointer count a released `select` as pressed.
const SELECT_PRESS: f32 = 0.7;

/// The `select` amount at or below which the menu and the game pointer count a pressed `select` as
/// released. The gap to [`SELECT_PRESS`] keeps a trigger that rests near one amount from pressing
/// again and again.
const SELECT_RELEASE: f32 = 0.5;

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

    /// Reads the controllers' input of the frame of `timing`, logs what they did and returns the
    /// frame. A failed read that is no loss of the session warns at most once per
    /// [`READ_FAILURE_PERIOD`] and counts as the idle input, since the screen works without
    /// controllers.
    ///
    /// # Errors
    ///
    /// Returns the [`XrError`] of a read that lost the session.
    pub(crate) fn read(
        &mut self,
        session: &Session,
        timing: FrameTiming,
    ) -> Result<Frame, XrError> {
        let (input, failure) = frame_input(session.read_input(timing))?;
        if let Some(error) = failure {
            warn_every!(
                self.read_failure,
                READ_FAILURE_PERIOD,
                error = %error,
                "the controllers could not be read, so they count as idle"
            );
        }
        let (left, right) = self.watch.update(&input, log);
        Ok(Frame { input, left, right })
    }
}

/// The controllers in one frame: the input, and each hand's buttons in the frame before and in
/// this frame.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Frame {
    pub(crate) input: Input,
    pub(crate) left: Buttons,
    pub(crate) right: Buttons,
}

impl Frame {
    pub(crate) const fn buttons(&self, hand: Hand) -> Buttons {
        match hand {
            Hand::Left => self.left,
            Hand::Right => self.right,
        }
    }
}

/// A hand's buttons in the frame before and in this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Buttons {
    pub(crate) before: HandState,
    pub(crate) now: HandState,
}

impl Buttons {
    /// Whether `menu` went from released to pressed in this frame.
    pub(crate) const fn menu_pressed(self) -> bool {
        rose(self.before.menu, self.now.menu)
    }

    /// Whether `select` went from released to pressed in this frame.
    pub(crate) const fn select_pressed(self) -> bool {
        rose(self.before.select, self.now.select)
    }
}

/// Whether a button went from released in `before` to pressed in `now`.
pub(crate) const fn rose(before: ButtonState, now: ButtonState) -> bool {
    matches!((before, now), (ButtonState::Released, ButtonState::Pressed))
}

/// Whether the menu and the game pointer count `select` as pressed at `amount`, after it counted as
/// `before`.
fn select_state(before: ButtonState, amount: f32) -> ButtonState {
    match before {
        ButtonState::Released if amount >= SELECT_PRESS => ButtonState::Pressed,
        ButtonState::Pressed if amount <= SELECT_RELEASE => ButtonState::Released,
        ButtonState::Released | ButtonState::Pressed => before,
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
pub(crate) enum Control {
    Menu,
    Select,
    Secondary,
}

impl Control {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Menu => "menu",
            Self::Select => "select",
            Self::Secondary => "secondary",
        }
    }
}

/// What a hand did since the frame before.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Change {
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

/// What one hand held in one frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct HandState {
    aim: Tracking,
    pub(crate) menu: ButtonState,
    /// `select` as the menu and the game pointer count it, by [`select_state`].
    pub(crate) select: ButtonState,
    /// `select` as the log counts it: pressed above [`SELECT_PRESS`].
    logged_select: ButtonState,
    pub(crate) secondary: ButtonState,
}

impl HandState {
    /// The state of `input`, after the hand held `before` in the frame before.
    fn of(input: &HandInput, before: Self) -> Self {
        let tracking = if input.aim.is_some() { Tracking::Tracked } else { Tracking::Lost };
        let logged_select =
            if input.select > SELECT_PRESS { ButtonState::Pressed } else { ButtonState::Released };
        Self {
            aim: tracking,
            menu: input.menu,
            select: select_state(before.select, input.select),
            logged_select,
            secondary: input.secondary,
        }
    }
}

/// The controllers' state of the frame before, which turns each frame's input into changes.
#[derive(Debug, Default)]
pub(crate) struct ControllerWatch {
    left: HandState,
    right: HandState,
}

impl ControllerWatch {
    /// Calls `on_change` with each change from the frame before to `input`, and returns the left
    /// and the right hand's buttons.
    pub(crate) fn update(
        &mut self,
        input: &Input,
        mut on_change: impl FnMut(Change),
    ) -> (Buttons, Buttons) {
        let mut step = |hand: Hand, held: &mut HandState| {
            let buttons = Buttons { before: *held, now: HandState::of(input.hand(hand), *held) };
            report(hand, buttons.before, buttons.now, &mut on_change);
            *held = buttons.now;
            buttons
        };
        let left = step(Hand::Left, &mut self.left);
        let right = step(Hand::Right, &mut self.right);
        (left, right)
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
        (Control::Select, before.logged_select, now.logged_select),
        (Control::Secondary, before.secondary, now.secondary),
    ];
    for (control, _, _) in buttons.into_iter().filter(|(_, was, is)| rose(*was, *is)) {
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
