//! The player's failure messages and the conversion of each launch and reload error.

use super::{Failure, timeout_text};
use diag::logging::LogError;
use diag::root::RootError;
use ipc_layout::launch_token::{Status, TokenError};
use launch::checks::{GameRunning, RuntimeError};
use launch::registry::{Hive, RegistryError, RegistryValue};
use launch::reload::ReloadError;
use launch::status::{Goal, WaitError};
use launch::token::TokenCreateError;
use std::io;
use std::path::PathBuf;
use std::time::Duration;

use crate::args::ArgsError;

fn log_error() -> LogError {
    let source = io::Error::new(io::ErrorKind::PermissionDenied, "access denied");
    LogError::CreateDir { path: PathBuf::from(r"C:\logs"), source }
}

#[test]
fn a_reload_with_no_running_game_says_so_and_names_the_fix() {
    let failure = Failure::from(ReloadError::NoGame);
    assert!(matches!(failure, Failure::NoGame), "{failure:?}");
    let expected = concat!(
        "No running game with Arcane Eye VR accepts a settings reload, so no settings were ",
        "reloaded. Start the game with Arcane Eye VR, then start arcane-eye-vr.exe ",
        "--reload-settings again. When the game already runs, the game's log in the log folder ",
        "names the cause, and the settings files apply when the game starts again. The log ",
        "folder is ",
        r"%ARCANE_EYE_VR_HOME%\logs when ARCANE_EYE_VR_HOME is an absolute path, and ",
        r"%LOCALAPPDATA%\arcane-eye-vr\logs otherwise."
    );
    assert_eq!(failure.to_string(), expected);
}

#[test]
fn a_reload_that_windows_refused_names_the_cause_and_the_restart() {
    let failure = Failure::from(ReloadError::Signal(windows::core::Error::empty()));
    assert!(matches!(failure, Failure::Reload(ReloadError::Signal(_))), "{failure:?}");
    let message = failure.to_string();
    let start = concat!(
        "Arcane Eye VR could not ask the game to reload its settings ",
        "(could not signal the settings reload event"
    );
    assert!(message.starts_with(start), "{message}");
    let end = "Quit the game and start it again to apply the settings files.";
    assert!(message.ends_with(end), "{message}");
    let open = Failure::from(ReloadError::Open(windows::core::Error::empty()));
    assert!(matches!(open, Failure::Reload(ReloadError::Open(_))), "{open:?}");
}

#[test]
fn an_argument_failure_names_every_command_line() {
    let message = Failure::from(ArgsError::ReloadWithMode).to_string();
    let expected = concat!(
        "arcane-eye-vr.exe got --reload-settings with --mode, and a reload starts no game. ",
        "Start it with no arguments for VR, with --mode desktop-stereo, with --mode screen, ",
        "or with --reload-settings while the game runs."
    );
    assert_eq!(message, expected);
}

#[test]
fn existing_token_is_its_own_failure() {
    assert!(matches!(Failure::from(TokenCreateError::Exists), Failure::TokenExists));
    let create = TokenCreateError::Create(windows::core::Error::empty());
    assert!(matches!(Failure::from(create), Failure::TokenCreate(_)));
}

#[test]
fn log_failure_message_names_the_cause_and_both_log_folders() {
    let expected = concat!(
        r"Arcane Eye VR could not start its log (could not create the log folder C:\logs: ",
        r"access denied). Check that the log folder can be written. The log folder is ",
        r"%ARCANE_EYE_VR_HOME%\logs when ARCANE_EYE_VR_HOME is an absolute path, and ",
        r"%LOCALAPPDATA%\arcane-eye-vr\logs otherwise."
    );
    assert_eq!(Failure::Log(log_error()).to_string(), expected);
}

#[test]
fn core_load_failure_message_points_at_both_log_folders() {
    let message = Failure::CoreLoadFailed.to_string();
    let end = concat!(
        r"The log in the log folder may name the cause. The log folder is ",
        r"%ARCANE_EYE_VR_HOME%\logs when ARCANE_EYE_VR_HOME is an absolute path, and ",
        r"%LOCALAPPDATA%\arcane-eye-vr\logs otherwise."
    );
    assert!(message.ends_with(end), "{message}");
}

#[test]
fn root_failure_message_names_the_cause_and_the_variable_that_avoids_it() {
    let root = RootError::LocalAppData(windows::core::Error::empty());
    let message = Failure::from(root).to_string();
    let start = concat!(
        "Arcane Eye VR could not find its folder ",
        "(could not resolve the local app data folder"
    );
    assert!(message.starts_with(start), "{message}");
    let end = concat!(
        "Set ARCANE_EYE_VR_HOME to an absolute folder path, ",
        "then start Arcane Eye VR again."
    );
    assert!(message.ends_with(end), "{message}");
}

#[test]
fn timeout_message_names_the_goal_the_last_status_and_its_fix() {
    let ten_minutes = Duration::from_mins(10);
    let vr = Failure::Timeout {
        goal: Goal::VrSessionRunning,
        last: Status::FirstPresent,
        timeout: ten_minutes,
    }
    .to_string();
    let vr_start = concat!(
        "The game did not report that the VR session runs within 10 minutes. The last status ",
        "was that the game drew its first frame. Check that the headset is connected and ",
        "SteamVR runs. The log in the log folder names the last step. The log folder is "
    );
    assert!(vr.starts_with(vr_start), "{vr}");
    assert!(vr.ends_with(r"%LOCALAPPDATA%\arcane-eye-vr\logs otherwise."), "{vr}");
    let desktop = Failure::Timeout {
        goal: Goal::FirstPresent,
        last: Status::Waiting,
        timeout: Duration::from_mins(3),
    };
    let desktop = desktop.to_string();
    let desktop_start = concat!(
        "The game did not report that the game drew its first frame within 3 minutes. The ",
        "last status was that the game has not loaded Arcane Eye VR. Check that Steam started ",
        "the game"
    );
    assert!(desktop.starts_with(desktop_start), "{desktop}");
}

#[test]
fn timeout_reads_in_whole_minutes_or_else_in_seconds_rounded_up() {
    assert_eq!(timeout_text(Duration::from_mins(10)), "10 minutes");
    assert_eq!(timeout_text(Duration::from_mins(1)), "1 minute");
    assert_eq!(timeout_text(Duration::from_secs(90)), "90 seconds");
    assert_eq!(timeout_text(Duration::from_secs(30)), "30 seconds");
    assert_eq!(timeout_text(Duration::from_millis(1500)), "2 seconds");
    assert_eq!(timeout_text(Duration::from_millis(500)), "1 second");
    assert_eq!(timeout_text(Duration::ZERO), "0 seconds");
}

#[test]
fn runtime_read_message_names_the_value_and_the_fix() {
    let value =
        RegistryValue { hive: Hive::LocalMachine, key: r"SOFTWARE\Khronos", name: "Active" };
    let error = RegistryError { value, source: windows::core::Error::empty() };
    let message = Failure::from(RuntimeError::Read(error)).to_string();
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

#[test]
fn check_errors_become_the_player_s_failures() {
    let active = Some(r"C:\oculus.json".to_owned());
    let not_steamvr = Failure::from(RuntimeError::NotSteamVr { active: active.clone() });
    assert!(matches!(not_steamvr, Failure::NotSteamVr { active: found } if found == active));
    assert!(matches!(Failure::from(GameRunning), Failure::GameRunning));
}

#[test]
fn wait_errors_become_the_player_s_failures() {
    assert!(matches!(Failure::from(WaitError::WrongRenderer), Failure::WrongRenderer));
    assert!(matches!(Failure::from(WaitError::CoreLoadFailed), Failure::CoreLoadFailed));
    let unsupported = Failure::from(WaitError::UnsupportedGameVersion);
    assert!(matches!(unsupported, Failure::UnsupportedGameVersion));
    assert!(matches!(Failure::from(WaitError::UnknownStatus(9)), Failure::UnknownStatus(9)));
    let token = Failure::from(WaitError::Token(TokenError::Magic { found: 0 }));
    assert!(matches!(token, Failure::Token(TokenError::Magic { found: 0 })));
    let timeout = WaitError::Timeout {
        goal: Goal::FirstPresent,
        last: Status::HooksInstalled,
        timeout: Duration::from_mins(10),
    };
    assert!(matches!(
        Failure::from(timeout),
        Failure::Timeout { goal: Goal::FirstPresent, last: Status::HooksInstalled, timeout }
            if timeout == Duration::from_mins(10)
    ));
}
