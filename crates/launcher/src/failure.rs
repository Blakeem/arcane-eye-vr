//! Every way a launch or a settings reload request fails. The player reads each message in a message
//! box, so it names the cause and the fix.

use std::time::Duration;

use diag::error_chain::chain;
use diag::logging::LogError;
use diag::root::{DEFAULT_FOLDER, LOG_FOLDER, RootError};
use ipc_layout::launch_token::{Status, TokenError};
use ipc_layout::root_folder::HOME_VARIABLE;
use launch::checks::{GameRunning, RuntimeError};
use launch::registry::RegistryError;
use launch::reload::ReloadError;
use launch::status::{Goal, WaitError};
use launch::steam::OpenUrlError;
use launch::token::TokenCreateError;
use viewer_client::HeaderViewError;

use crate::args::{ArgsError, LaunchMode};
use crate::screen::failure::ScreenFailure;
use crate::start::StartError;

const SECONDS_PER_MINUTE: u64 = 60;

const STEAMVR_FIX: &str =
    "In SteamVR settings, open OpenXR and choose Set SteamVR as OpenXR runtime.";

/// Why the launch app stopped before the game reported its goal status, before the headset
/// screen's end, or before its settings reload request reached the game.
#[derive(Debug, thiserror::Error)]
pub(crate) enum Failure {
    #[error(
        "{0}. Start it with no arguments for VR, with --mode desktop-stereo, with --mode screen, \
         or with --reload-settings while the game runs."
    )]
    Args(#[from] ArgsError),
    #[error("{cause} {}", cause.fix(mode.as_str()))]
    Headset { cause: ScreenFailure, mode: LaunchMode },
    #[error(
        "Arcane Eye VR could not open the game's frames ({}). Quit the game, then start Arcane \
         Eye VR again.",
        chain(.0)
    )]
    Link(#[from] HeaderViewError),
    #[error(
        "No running game with Arcane Eye VR accepts a settings reload, so no settings were \
         reloaded. Start the game with Arcane Eye VR, then start arcane-eye-vr.exe \
         --reload-settings again. When the game already runs, the game's log in the log folder \
         names the cause, and the settings files apply when the game starts again. {}",
        log_folder_text()
    )]
    NoGame,
    #[error(
        "Arcane Eye VR could not ask the game to reload its settings ({}). Quit the game and start \
         it again to apply the settings files.",
        chain(.0)
    )]
    Reload(#[source] ReloadError),
    #[error(
        "Arcane Eye VR could not find its folder ({}). Set {HOME_VARIABLE} to an absolute folder \
         path, then start Arcane Eye VR again.",
        chain(.0)
    )]
    Root(#[from] RootError),
    #[error(
        "Arcane Eye VR could not start its log ({}). Check that the log folder can be written. {}",
        chain(.0),
        log_folder_text()
    )]
    Log(#[from] LogError),
    #[error("Arcane Eye VR could not read the active OpenXR runtime ({}). {STEAMVR_FIX}", chain(.0))]
    RuntimeRead(#[source] RegistryError),
    #[error(
        "SteamVR is not the active OpenXR runtime. The active runtime is {}. {STEAMVR_FIX}",
        active.as_deref().unwrap_or("not set")
    )]
    NotSteamVr { active: Option<String> },
    #[error("Baldur's Gate 3 is already running. Quit the game, then start Arcane Eye VR again.")]
    GameRunning,
    #[error(
        "Arcane Eye VR is already waiting for the game from an earlier launch. Wait for that launch to \
         finish, or end arcane-eye-vr.exe in Task Manager."
    )]
    TokenExists,
    #[error(
        "Arcane Eye VR could not create its launch token ({}). Restart Windows, then start Arcane Eye VR \
         again.",
        chain(.0)
    )]
    TokenCreate(#[source] TokenCreateError),
    #[error("{0}. Check that Steam is installed.")]
    SteamOpen(#[from] OpenUrlError),
    #[error(
        "The game started with the Vulkan renderer. Choose DirectX 11 in the Larian launcher, then \
         start Arcane Eye VR again."
    )]
    WrongRenderer,
    #[error(
        "The game could not start Arcane Eye VR. Check that arcane_eye_core.dll is in the game's bin folder \
         and that the virus scanner did not remove it. The log in the log folder may name the cause. {}",
        log_folder_text()
    )]
    CoreLoadFailed,
    #[error(
        "Arcane Eye VR does not support this game version. Install the Arcane Eye VR release that \
         supports it."
    )]
    UnsupportedGameVersion,
    #[error(
        "The game reported status {0}, and this version of arcane-eye-vr.exe does not know it. \
         Install arcane-eye-vr.exe and the mod files from the same release."
    )]
    UnknownStatus(u32),
    #[error(
        "The launch token holds data that Arcane Eye VR did not write ({0}). Close other Arcane Eye VR \
         tools, then start Arcane Eye VR again."
    )]
    Token(#[source] TokenError),
    #[error(
        "The game did not report that {} within {}. The last status was that \
         {}. {} The log \
         in the log folder names the last step. {}",
        status_text(.goal.status()),
        timeout_text(*.timeout),
        status_text(*.last),
        timeout_fix(*.last),
        log_folder_text()
    )]
    Timeout { goal: Goal, last: Status, timeout: Duration },
}

impl From<RuntimeError> for Failure {
    fn from(error: RuntimeError) -> Self {
        match error {
            RuntimeError::Read(source) => Self::RuntimeRead(source),
            RuntimeError::NotSteamVr { active } => Self::NotSteamVr { active },
        }
    }
}

impl From<ReloadError> for Failure {
    fn from(error: ReloadError) -> Self {
        match error {
            ReloadError::NoGame => Self::NoGame,
            ReloadError::Open(_) | ReloadError::Signal(_) => Self::Reload(error),
        }
    }
}

impl From<GameRunning> for Failure {
    fn from(_: GameRunning) -> Self {
        Self::GameRunning
    }
}

impl From<TokenCreateError> for Failure {
    fn from(error: TokenCreateError) -> Self {
        match error {
            TokenCreateError::Exists => Self::TokenExists,
            TokenCreateError::Create(_) | TokenCreateError::Map(_) => Self::TokenCreate(error),
        }
    }
}

impl From<StartError> for Failure {
    fn from(error: StartError) -> Self {
        match error {
            StartError::Token(source) => source.into(),
            StartError::Steam(source) => source.into(),
            StartError::Wait(source) => source.into(),
        }
    }
}

impl From<WaitError> for Failure {
    fn from(error: WaitError) -> Self {
        match error {
            WaitError::WrongRenderer => Self::WrongRenderer,
            WaitError::CoreLoadFailed => Self::CoreLoadFailed,
            WaitError::UnsupportedGameVersion => Self::UnsupportedGameVersion,
            WaitError::UnknownStatus(value) => Self::UnknownStatus(value),
            WaitError::Token(source) => Self::Token(source),
            WaitError::Timeout { goal, last, timeout } => Self::Timeout { goal, last, timeout },
        }
    }
}

/// Where the log folder is, as the player reads it. It names both folders, since the game's core
/// reads the root folder variable from Steam's environment and not from the launch app's.
fn log_folder_text() -> String {
    format!(
        "The log folder is %{HOME_VARIABLE}%\\{LOG_FOLDER} when {HOME_VARIABLE} is an absolute path, \
         and %LOCALAPPDATA%\\{DEFAULT_FOLDER}\\{LOG_FOLDER} otherwise."
    )
}

/// A status as the player reads it.
const fn status_text(status: Status) -> &'static str {
    match status {
        Status::Waiting => "the game has not loaded Arcane Eye VR",
        Status::CoreLoading => "Arcane Eye VR is loading",
        Status::HooksInstalled => "Arcane Eye VR waits for the first frame",
        Status::FirstPresent => "the game drew its first frame",
        Status::VrSessionRunning => "the VR session runs",
        Status::WrongRenderer => "the game runs the Vulkan renderer",
        Status::CoreLoadFailed => "the game could not start Arcane Eye VR",
        Status::SignatureFailure => "Arcane Eye VR does not support this game version",
    }
}

/// The fix for a launch that timed out at `last`.
const fn timeout_fix(last: Status) -> &'static str {
    match last {
        Status::Waiting => {
            "Check that Steam started the game and that you clicked Play in the Larian launcher."
        }
        Status::FirstPresent => "Check that the headset is connected and SteamVR runs.",
        Status::CoreLoading
        | Status::HooksInstalled
        | Status::VrSessionRunning
        | Status::WrongRenderer
        | Status::CoreLoadFailed
        | Status::SignatureFailure => "Quit the game, then start Arcane Eye VR again.",
    }
}

/// A timeout as the player reads it, in whole minutes or else in seconds rounded up.
fn timeout_text(timeout: Duration) -> String {
    let seconds = timeout.as_secs().saturating_add(u64::from(timeout.subsec_nanos() > 0));
    let (count, unit) = if seconds != 0 && seconds.is_multiple_of(SECONDS_PER_MINUTE) {
        (seconds / SECONDS_PER_MINUTE, "minute")
    } else {
        (seconds, "second")
    };
    let plural = if count == 1 { "" } else { "s" };
    format!("{count} {unit}{plural}")
}

#[cfg(test)]
mod tests;
