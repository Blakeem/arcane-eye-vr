//! The launch steps' decisions as pure functions: the runtime check, the running-game check and what
//! each status poll does.

use std::path::Path;
use std::time::Duration;

use ipc_layout::launch_token::{LaunchToken, Status, TokenError, UnknownStatus};

use crate::checks::{GameRunning, RuntimeError};
use crate::registry::RegistryError;
use crate::status::{Goal, WaitError};

/// The manifest file name SteamVR registers as the active OpenXR runtime.
const STEAMVR_RUNTIME_FILE: &str = "steamxr_win64.json";

/// What a wait does after one status poll.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Poll {
    Wait,
    Done,
}

/// Checks that `active_runtime`, the registry's active OpenXR runtime manifest path, is SteamVR's.
/// Windows file names ignore case.
pub(crate) fn check_runtime(active_runtime: Option<String>) -> Result<(), RuntimeError> {
    let file_name = active_runtime
        .as_deref()
        .and_then(|path| Path::new(path).file_name())
        .and_then(|name| name.to_str());
    if file_name.is_some_and(|name| name.eq_ignore_ascii_case(STEAMVR_RUNTIME_FILE)) {
        return Ok(());
    }
    Err(RuntimeError::NotSteamVr { active: active_runtime })
}

/// Whether `running`, Steam's `Running` value for the game, says it runs. A missing value means it
/// does not.
pub(crate) const fn reports_running(running: Option<u32>) -> bool {
    matches!(running, Some(value) if value != 0)
}

/// Fails when `running`, the read of Steam's `Running` value for the game, says it runs. A failed
/// read lets the launch go ahead, since the value is undocumented and only guards against a second
/// game start.
pub(crate) const fn check_not_running(
    running: &Result<Option<u32>, RegistryError>,
) -> Result<(), GameRunning> {
    match running {
        Ok(value) if reports_running(*value) => Err(GameRunning),
        Ok(_) | Err(_) => Ok(()),
    }
}

/// What a wait for `goal` does with `token`, read `elapsed` after the wait started. The wait fails
/// once `elapsed` reaches `timeout`.
pub(crate) fn poll(
    goal: Goal,
    token: Result<LaunchToken, TokenError>,
    elapsed: Duration,
    timeout: Duration,
) -> Result<Poll, WaitError> {
    let status = match token?.status() {
        Ok(status) => status,
        Err(UnknownStatus(value)) => return Err(WaitError::UnknownStatus(value)),
    };
    let poll = status_poll(goal, status)?;
    if poll == Poll::Wait && elapsed >= timeout {
        return Err(WaitError::Timeout { goal, last: status, timeout });
    }
    Ok(poll)
}

fn status_poll(goal: Goal, status: Status) -> Result<Poll, WaitError> {
    match status {
        Status::Waiting | Status::CoreLoading | Status::HooksInstalled => Ok(Poll::Wait),
        Status::FirstPresent => {
            Ok(if goal == Goal::FirstPresent { Poll::Done } else { Poll::Wait })
        }
        Status::VrSessionRunning => Ok(Poll::Done),
        Status::WrongRenderer => Err(WaitError::WrongRenderer),
        Status::CoreLoadFailed => Err(WaitError::CoreLoadFailed),
        Status::SignatureFailure => Err(WaitError::UnsupportedGameVersion),
    }
}

#[cfg(test)]
mod tests {
    use super::{Poll, check_not_running, check_runtime, poll, reports_running};
    use crate::checks::{GameRunning, RuntimeError};
    use crate::registry::{Hive, RegistryError, RegistryValue};
    use crate::status::{Goal, WaitError};
    use ipc_layout::launch_token::{LaunchToken, Mode, STATUS_OFFSET, Status, TokenError};
    use std::time::Duration;

    const STARTED: Duration = Duration::ZERO;
    const TIMEOUT: Duration = Duration::from_mins(10);
    const LATE: Duration = Duration::from_secs(599);

    fn token(status: u32) -> Result<LaunchToken, TokenError> {
        let mut bytes = LaunchToken::new(Mode::Vr, 1, 2).encode();
        bytes[STATUS_OFFSET..STATUS_OFFSET + 4].copy_from_slice(&status.to_le_bytes());
        LaunchToken::decode(&bytes)
    }

    fn runtime(path: &str) -> Result<(), RuntimeError> {
        check_runtime(Some(path.to_owned()))
    }

    fn poll_at(goal: Goal, status: u32, elapsed: Duration) -> Result<Poll, WaitError> {
        poll(goal, token(status), elapsed, TIMEOUT)
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
                matches!(&result, Err(RuntimeError::NotSteamVr { active: Some(active) }) if active == path),
                "{path}: {result:?}"
            );
        }
        assert!(matches!(check_runtime(None), Err(RuntimeError::NotSteamVr { active: None })));
    }

    #[test]
    fn running_value_one_or_any_nonzero_fails_the_launch() {
        for value in [1, 7] {
            assert_eq!(check_not_running(&Ok(Some(value))), Err(GameRunning));
        }
        assert_eq!(check_not_running(&Ok(Some(0))), Ok(()));
        assert_eq!(check_not_running(&Ok(None)), Ok(()));
    }

    #[test]
    fn steam_reports_the_game_running_for_a_nonzero_value_only() {
        assert!(reports_running(Some(1)), "value 1");
        assert!(reports_running(Some(7)), "value 7");
        assert!(!reports_running(Some(0)), "value 0");
        assert!(!reports_running(None), "no value");
    }

    #[test]
    fn running_value_read_error_lets_the_launch_go_ahead() {
        let value =
            RegistryValue { hive: Hive::CurrentUser, key: r"Software\Valve", name: "Running" };
        let read = Err(RegistryError { value, source: windows::core::Error::empty() });
        assert_eq!(check_not_running(&read), Ok(()));
    }

    #[test]
    fn progress_statuses_wait_for_both_goals() {
        for goal in [Goal::VrSessionRunning, Goal::FirstPresent] {
            for status in [0, 1, 2] {
                assert_eq!(poll_at(goal, status, STARTED).ok(), Some(Poll::Wait), "{status}");
            }
        }
    }

    #[test]
    fn first_present_goal_is_done_at_first_present_and_vr_goal_at_session_running() {
        assert_eq!(poll_at(Goal::FirstPresent, 3, STARTED).ok(), Some(Poll::Done));
        assert_eq!(poll_at(Goal::VrSessionRunning, 3, STARTED).ok(), Some(Poll::Wait));
        assert_eq!(poll_at(Goal::VrSessionRunning, 4, STARTED).ok(), Some(Poll::Done));
        assert_eq!(poll_at(Goal::FirstPresent, 4, STARTED).ok(), Some(Poll::Done));
    }

    #[test]
    fn failure_statuses_fail_at_once_for_both_goals() {
        for goal in [Goal::VrSessionRunning, Goal::FirstPresent] {
            assert!(matches!(poll_at(goal, 10, STARTED), Err(WaitError::WrongRenderer)));
            assert!(matches!(poll_at(goal, 11, STARTED), Err(WaitError::CoreLoadFailed)));
            assert!(matches!(poll_at(goal, 12, STARTED), Err(WaitError::UnsupportedGameVersion)));
        }
    }

    #[test]
    fn unknown_status_and_invalid_token_fail() {
        let vr = Goal::VrSessionRunning;
        assert!(matches!(poll_at(vr, 5, STARTED), Err(WaitError::UnknownStatus(5))));
        let wrong_magic = Err(TokenError::Magic { found: 0 });
        assert!(matches!(poll(vr, wrong_magic, STARTED, TIMEOUT), Err(WaitError::Token(_))));
    }

    #[test]
    fn waiting_times_out_at_the_given_timeout_with_the_last_status() {
        let vr = Goal::VrSessionRunning;
        assert_eq!(poll_at(vr, 3, LATE).ok(), Some(Poll::Wait));
        let timed_out = poll_at(vr, 3, TIMEOUT);
        assert!(
            matches!(
                timed_out,
                Err(WaitError::Timeout { goal: Goal::VrSessionRunning, last: Status::FirstPresent, timeout })
                    if timeout == Duration::from_secs(600)
            ),
            "{timed_out:?}"
        );
        let first_present = poll_at(Goal::FirstPresent, 0, Duration::from_secs(3600));
        assert!(matches!(first_present, Err(WaitError::Timeout { last: Status::Waiting, .. })));
        let short =
            poll(Goal::FirstPresent, token(2), Duration::from_secs(180), Duration::from_secs(180));
        assert!(matches!(short, Err(WaitError::Timeout { last: Status::HooksInstalled, .. })));
    }

    #[test]
    fn a_final_status_wins_over_the_timeout() {
        assert_eq!(poll_at(Goal::VrSessionRunning, 4, TIMEOUT).ok(), Some(Poll::Done));
        let failed = poll_at(Goal::VrSessionRunning, 11, TIMEOUT);
        assert!(matches!(failed, Err(WaitError::CoreLoadFailed)));
    }
}
