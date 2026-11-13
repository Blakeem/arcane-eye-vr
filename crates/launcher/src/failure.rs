//! Every way a launch fails. The player reads each message in a message box, so it names the cause
//! and the fix.

use std::error::Error;

use diag::logging::LogError;
use ipc_layout::launch_token::{Mode, Status, TokenError};

use crate::args::ArgsError;
use crate::decide::{TIMEOUT_MINUTES, goal};
use crate::raw::{OpenUrlError, TokenCreateError};
use crate::registry::RegistryError;

const LOG_FOLDER: &str = r"%LOCALAPPDATA%\arcane-eye-vr\logs";

const STEAMVR_FIX: &str =
    "In SteamVR settings, open OpenXR and choose Set SteamVR as OpenXR runtime.";

/// Why the launch app stopped before the game reported its goal status.
#[derive(Debug, thiserror::Error)]
pub(crate) enum Failure {
    #[error("{0}. Start it with no arguments for VR, or with --mode desktop-stereo.")]
    Args(#[from] ArgsError),
    #[error("Arcane Eye VR could not start its log ({}). Check that {LOG_FOLDER} can be written.", chain(.0))]
    Log(#[from] LogError),
    #[error("Arcane Eye VR could not read the active OpenXR runtime ({}). {STEAMVR_FIX}", chain(.0))]
    RuntimeRead(#[from] RegistryError),
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
         and that the virus scanner did not remove it. The log in {LOG_FOLDER} may name the cause."
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
    Token(#[from] TokenError),
    #[error(
        "The game did not report that {} within {TIMEOUT_MINUTES} minutes. The last status was that \
         {}. {} The log \
         in {LOG_FOLDER} names the last step.",
        status_text(goal(*.mode)),
        status_text(*.last),
        timeout_fix(*.last)
    )]
    Timeout { mode: Mode, last: Status },
}

impl From<TokenCreateError> for Failure {
    fn from(error: TokenCreateError) -> Self {
        match error {
            TokenCreateError::Exists => Self::TokenExists,
            TokenCreateError::Create(_) | TokenCreateError::Map(_) => Self::TokenCreate(error),
        }
    }
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

/// `error` and each error below it, joined by ": ", since a message box shows no source chain.
fn chain(error: &dyn Error) -> String {
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        text.push_str(": ");
        text.push_str(&cause.to_string());
        source = cause.source();
    }
    text
}

#[cfg(test)]
mod tests {
    use super::{Failure, chain};
    use crate::raw::TokenCreateError;
    use crate::registry::{Hive, RegistryError, RegistryValue};
    use diag::logging::LogError;
    use ipc_layout::launch_token::{Mode, Status};
    use std::io;
    use std::path::PathBuf;

    fn log_error() -> LogError {
        let source = io::Error::new(io::ErrorKind::PermissionDenied, "access denied");
        LogError::CreateDir { path: PathBuf::from(r"C:\logs"), source }
    }

    #[test]
    fn existing_token_is_its_own_failure() {
        assert!(matches!(Failure::from(TokenCreateError::Exists), Failure::TokenExists));
        let create = TokenCreateError::Create(windows::core::Error::empty());
        assert!(matches!(Failure::from(create), Failure::TokenCreate(_)));
    }

    #[test]
    fn chain_joins_each_cause_below_the_error() {
        assert_eq!(chain(&log_error()), r"could not create the log folder C:\logs: access denied");
    }

    #[test]
    fn log_failure_message_names_the_cause_and_the_folder() {
        let expected = concat!(
            r"Arcane Eye VR could not start its log (could not create the log folder C:\logs: ",
            r"access denied). Check that %LOCALAPPDATA%\arcane-eye-vr\logs can be written."
        );
        assert_eq!(Failure::Log(log_error()).to_string(), expected);
    }

    #[test]
    fn timeout_message_names_the_goal_the_last_status_and_its_fix() {
        let vr = Failure::Timeout { mode: Mode::Vr, last: Status::FirstPresent }.to_string();
        let vr_start = concat!(
            "The game did not report that the VR session runs within 10 minutes. The last status ",
            "was that the game drew its first frame. Check that the headset is connected and ",
            "SteamVR runs. The log in "
        );
        assert!(vr.starts_with(vr_start), "{vr}");
        let desktop = Failure::Timeout { mode: Mode::DesktopStereo, last: Status::Waiting };
        let desktop = desktop.to_string();
        let desktop_start = concat!(
            "The game did not report that the game drew its first frame within 10 minutes. The ",
            "last status was that the game has not loaded Arcane Eye VR. Check that Steam started ",
            "the game"
        );
        assert!(desktop.starts_with(desktop_start), "{desktop}");
    }

    #[test]
    fn runtime_read_message_names_the_value_and_the_fix() {
        let value =
            RegistryValue { hive: Hive::LocalMachine, key: r"SOFTWARE\Khronos", name: "Active" };
        let error = RegistryError { value, source: windows::core::Error::empty() };
        let message = Failure::from(error).to_string();
        let start = concat!(
            "Arcane Eye VR could not read the active OpenXR runtime ",
            r"(could not read HKLM\SOFTWARE\Khronos\Active"
        );
        assert!(message.starts_with(start), "{message}");
        assert!(message.ends_with("choose Set SteamVR as OpenXR runtime."), "{message}");
    }

    #[test]
    fn runtime_message_names_the_active_runtime_or_its_absence() {
        let other = Failure::NotSteamVr { active: Some(r"C:\oculus.json".to_owned()) };
        assert!(other.to_string().contains(r"The active runtime is C:\oculus.json. In SteamVR"));
        let unset = Failure::NotSteamVr { active: None };
        assert!(unset.to_string().contains("The active runtime is not set. In SteamVR"));
    }
}
