//! The launch app's decisions as pure functions: the runtime check, the running-game check, the
//! goal status of each mode and what each status poll does.

use std::path::Path;
use std::time::Duration;

use ipc_layout::launch_token::{LaunchToken, Mode, Status, TokenError, UnknownStatus};

use crate::failure::Failure;
use crate::registry::RegistryError;

/// The time between two reads of the launch token's status.
pub(crate) const POLL_INTERVAL: Duration = Duration::from_millis(250);

/// The longest wait for the goal status, in minutes. Steam can hold a launch for an update or a
/// cloud sync.
pub(crate) const TIMEOUT_MINUTES: u64 = 10;

pub(crate) const TIMEOUT: Duration = Duration::from_mins(TIMEOUT_MINUTES);

/// The manifest file name SteamVR registers as the active OpenXR runtime.
const STEAMVR_RUNTIME_FILE: &str = "steamxr_win64.json";

/// What the launch app does after one status poll.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Poll {
    Wait,
    Done,
}

/// Checks that `active_runtime`, the registry's active OpenXR runtime manifest path, is SteamVR's.
/// Windows file names ignore case.
pub(crate) fn check_runtime(active_runtime: Option<String>) -> Result<(), Failure> {
    let file_name = active_runtime
        .as_deref()
        .and_then(|path| Path::new(path).file_name())
        .and_then(|name| name.to_str());
    if file_name.is_some_and(|name| name.eq_ignore_ascii_case(STEAMVR_RUNTIME_FILE)) {
        return Ok(());
    }
    Err(Failure::NotSteamVr { active: active_runtime })
}

/// Fails when `running`, Steam's `Running` value for the game, says it runs. A missing value means
/// it does not. A failed read lets the launch go ahead, since the value is undocumented and only
/// guards against a second game start.
pub(crate) const fn check_not_running(
    running: &Result<Option<u32>, RegistryError>,
) -> Result<(), Failure> {
    match running {
        Ok(Some(value)) if *value != 0 => Err(Failure::GameRunning),
        Ok(_) | Err(_) => Ok(()),
    }
}

/// The status at which a launch in `mode` is done.
pub(crate) const fn goal(mode: Mode) -> Status {
    match mode {
        Mode::DesktopStereo => Status::FirstPresent,
        Mode::Vr => Status::VrSessionRunning,
    }
}

/// What the launch app does with `token`, read `elapsed` after the game was started in `mode`.
pub(crate) fn poll(
    mode: Mode,
    token: Result<LaunchToken, TokenError>,
    elapsed: Duration,
) -> Result<Poll, Failure> {
    let status = match token?.status() {
        Ok(status) => status,
        Err(UnknownStatus(value)) => return Err(Failure::UnknownStatus(value)),
    };
    let poll = status_poll(mode, status)?;
    if poll == Poll::Wait && elapsed >= TIMEOUT {
        return Err(Failure::Timeout { mode, last: status });
    }
    Ok(poll)
}

fn status_poll(mode: Mode, status: Status) -> Result<Poll, Failure> {
    match status {
        Status::Waiting | Status::CoreLoading | Status::HooksInstalled => Ok(Poll::Wait),
        Status::FirstPresent => {
            Ok(if goal(mode) == Status::FirstPresent { Poll::Done } else { Poll::Wait })
        }
        Status::VrSessionRunning => Ok(Poll::Done),
        Status::WrongRenderer => Err(Failure::WrongRenderer),
        Status::CoreLoadFailed => Err(Failure::CoreLoadFailed),
        Status::SignatureFailure => Err(Failure::UnsupportedGameVersion),
    }
}

#[cfg(test)]
mod tests {
    use super::{Poll, TIMEOUT, check_not_running, check_runtime, poll};
    use crate::failure::Failure;
    use crate::registry::{Hive, RegistryError, RegistryValue};
    use ipc_layout::launch_token::{LaunchToken, Mode, STATUS_OFFSET, Status, TokenError};
    use std::time::Duration;

    const STARTED: Duration = Duration::ZERO;
    const LATE: Duration = Duration::from_secs(599);

    fn token(status: u32) -> Result<LaunchToken, TokenError> {
        let mut bytes = LaunchToken::new(Mode::Vr, 1, 2).encode();
        bytes[STATUS_OFFSET..STATUS_OFFSET + 4].copy_from_slice(&status.to_le_bytes());
        LaunchToken::decode(&bytes)
    }

    fn runtime(path: &str) -> Result<(), Failure> {
        check_runtime(Some(path.to_owned()))
    }

    #[test]
    fn steamvr_manifest_passes_the_runtime_check_in_any_case() {
        let steam = r"C:\Program Files (x86)\Steam\steamapps\common\SteamVR\steamxr_win64.json";
        for path in [
            steam,
            r"D:\SteamVR\SteamXR_Win64.JSON",
            "D:/SteamVR/steamxr_win64.json",
            "steamxr_win64.json",
        ] {
            assert!(matches!(runtime(path), Ok(())), "{path}");
        }
    }

    #[test]
    fn other_or_missing_runtimes_fail_with_the_active_path() {
        for path in [
            r"C:\Program Files\Oculus\Support\oculus-runtime\oculus_openxr_64.json",
            r"C:\Windows\system32\MixedRealityRuntime.json",
            r"C:\SteamVR\steamxr_win64.json.bak",
            r"C:\SteamVR\steamxr_win32.json",
            r"C:\SteamVR\",
            "",
        ] {
            let result = runtime(path);
            assert!(
                matches!(&result, Err(Failure::NotSteamVr { active: Some(active) }) if active == path),
                "{path}: {result:?}"
            );
        }
        assert!(matches!(check_runtime(None), Err(Failure::NotSteamVr { active: None })));
    }

    #[test]
    fn running_value_one_or_any_nonzero_fails_the_launch() {
        for value in [1, 7] {
            assert!(matches!(check_not_running(&Ok(Some(value))), Err(Failure::GameRunning)));
        }
        assert!(matches!(check_not_running(&Ok(Some(0))), Ok(())));
        assert!(matches!(check_not_running(&Ok(None)), Ok(())));
    }

    #[test]
    fn running_value_read_error_lets_the_launch_go_ahead() {
        let value =
            RegistryValue { hive: Hive::CurrentUser, key: r"Software\Valve", name: "Running" };
        let read = Err(RegistryError { value, source: windows::core::Error::empty() });
        assert!(matches!(check_not_running(&read), Ok(())));
    }

    #[test]
    fn progress_statuses_wait_in_both_modes() {
        for mode in [Mode::Vr, Mode::DesktopStereo] {
            for status in [0, 1, 2] {
                assert_eq!(poll(mode, token(status), STARTED).ok(), Some(Poll::Wait), "{status}");
            }
        }
    }

    #[test]
    fn desktop_stereo_is_done_at_first_present_and_vr_at_session_running() {
        assert_eq!(poll(Mode::DesktopStereo, token(3), STARTED).ok(), Some(Poll::Done));
        assert_eq!(poll(Mode::Vr, token(3), STARTED).ok(), Some(Poll::Wait));
        assert_eq!(poll(Mode::Vr, token(4), STARTED).ok(), Some(Poll::Done));
        assert_eq!(poll(Mode::DesktopStereo, token(4), STARTED).ok(), Some(Poll::Done));
    }

    #[test]
    fn failure_statuses_fail_at_once_in_both_modes() {
        for mode in [Mode::Vr, Mode::DesktopStereo] {
            assert!(matches!(poll(mode, token(10), STARTED), Err(Failure::WrongRenderer)));
            assert!(matches!(poll(mode, token(11), STARTED), Err(Failure::CoreLoadFailed)));
            assert!(matches!(poll(mode, token(12), STARTED), Err(Failure::UnsupportedGameVersion)));
        }
    }

    #[test]
    fn unknown_status_and_invalid_token_fail() {
        assert!(matches!(poll(Mode::Vr, token(5), STARTED), Err(Failure::UnknownStatus(5))));
        let wrong_magic = Err(TokenError::Magic { found: 0 });
        assert!(matches!(poll(Mode::Vr, wrong_magic, STARTED), Err(Failure::Token(_))));
    }

    #[test]
    fn waiting_times_out_at_ten_minutes_with_the_last_status() {
        assert_eq!(poll(Mode::Vr, token(3), LATE).ok(), Some(Poll::Wait));
        let timed_out = poll(Mode::Vr, token(3), TIMEOUT);
        let expected_last = Status::FirstPresent;
        assert!(
            matches!(timed_out, Err(Failure::Timeout { mode: Mode::Vr, last }) if last == expected_last),
            "{timed_out:?}"
        );
        let desktop = poll(Mode::DesktopStereo, token(0), Duration::from_secs(3600));
        assert!(matches!(desktop, Err(Failure::Timeout { last: Status::Waiting, .. })));
    }

    #[test]
    fn a_final_status_wins_over_the_timeout() {
        assert_eq!(poll(Mode::Vr, token(4), TIMEOUT).ok(), Some(Poll::Done));
        assert!(matches!(poll(Mode::Vr, token(11), TIMEOUT), Err(Failure::CoreLoadFailed)));
    }
}
