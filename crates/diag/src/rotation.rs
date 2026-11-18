//! Session log file names, their rotation at start and the prune of fallback logs.

use std::cmp::Reverse;
use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::logging::LogName;

const PREVIOUS_SESSIONS: u32 = 4;
const FALLBACK_INFIX: &str = ".pid";
const LOG_EXTENSION: &str = ".log";

/// The `log` file of `index` sessions ago. Index 0 is the current session.
pub(crate) fn session_path(dir: &Path, log: LogName, index: u32) -> PathBuf {
    let stem = log.stem();
    if index == 0 {
        dir.join(format!("{stem}{LOG_EXTENSION}"))
    } else {
        dir.join(format!("{stem}.{index}{LOG_EXTENSION}"))
    }
}

/// The `log` file of a session whose rotation failed. The process id keeps it apart from the
/// numbered files, which another process may hold open.
pub(crate) fn fallback_path(dir: &Path, log: LogName, process_id: u32) -> PathBuf {
    dir.join(format!("{}{FALLBACK_INFIX}{process_id}{LOG_EXTENSION}", log.stem()))
}

fn is_fallback_name(log: LogName, name: &OsStr) -> bool {
    let process_id = name
        .to_str()
        .and_then(|text| text.strip_prefix(log.stem()))
        .and_then(|rest| rest.strip_prefix(FALLBACK_INFIX))
        .and_then(|rest| rest.strip_suffix(LOG_EXTENSION));
    process_id.is_some_and(|id| !id.is_empty() && id.bytes().all(|byte| byte.is_ascii_digit()))
}

/// The fallback session log that a prune could not list or remove.
#[derive(Debug)]
pub(crate) struct PruneError {
    pub(crate) path: PathBuf,
    pub(crate) source: io::Error,
}

/// Removes the oldest fallback files of `log`, so at most `PREVIOUS_SESSIONS` of them stay.
///
/// The numbered rotation never moves a fallback log, so without this each failed rotation leaves
/// one more file for good.
pub(crate) fn prune_fallbacks(dir: &Path, log: LogName) -> Result<(), PruneError> {
    let keep = usize::try_from(PREVIOUS_SESSIONS).unwrap_or(usize::MAX);
    let mut logs = fallback_logs(dir, log)?;
    logs.sort_by_key(|(written, _)| Reverse(*written));
    for (_, path) in logs.into_iter().skip(keep) {
        match fs::remove_file(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(source) => return Err(PruneError { path, source }),
        }
    }
    Ok(())
}

/// Every fallback file of `log` in `dir`, with its last write time.
fn fallback_logs(dir: &Path, log: LogName) -> Result<Vec<(SystemTime, PathBuf)>, PruneError> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(source) => return Err(PruneError { path: dir.to_path_buf(), source }),
    };
    let mut logs = Vec::new();
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(source) => return Err(PruneError { path: dir.to_path_buf(), source }),
        };
        if !is_fallback_name(log, &entry.file_name()) {
            continue;
        }
        let path = entry.path();
        match entry.metadata().and_then(|metadata| metadata.modified()) {
            Ok(modified) => logs.push((modified, path)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(source) => return Err(PruneError { path, source }),
        }
    }
    Ok(logs)
}

/// The rename that stopped a rotation.
#[derive(Debug)]
pub(crate) struct RotateError {
    pub(crate) from: PathBuf,
    pub(crate) to: PathBuf,
    pub(crate) source: io::Error,
}

/// Shifts each file of `log` one index older and drops the one past `PREVIOUS_SESSIONS`.
///
/// It stops at the first failed rename, so no session log is overwritten by a younger one.
pub(crate) fn rotate(dir: &Path, log: LogName) -> Result<(), RotateError> {
    for index in (1..=PREVIOUS_SESSIONS).rev() {
        let from = session_path(dir, log, index.saturating_sub(1));
        let to = session_path(dir, log, index);
        match fs::rename(&from, &to) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(source) => return Err(RotateError { from, to, source }),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{fallback_path, prune_fallbacks, rotate, session_path};
    use crate::logging::LogName;
    use crate::test_dir;
    use std::fs;
    use std::os::windows::fs::OpenOptionsExt as _;
    use std::time::{Duration, SystemTime};

    #[test]
    fn rotation_keeps_four_previous_sessions() {
        let folder = test_dir("rotate-full");
        let dir = folder.path();
        for (index, text) in ["s0", "s1", "s2", "s3", "s4"].into_iter().enumerate() {
            let index = u32::try_from(index).expect("small index");
            fs::write(session_path(dir, LogName::Core, index), text).expect("write a session log");
        }
        rotate(dir, LogName::Core).expect("rotate");
        assert!(!dir.join("arcane-eye-vr.log").exists());
        let kept: Vec<String> = (1..=4)
            .map(|index| {
                fs::read_to_string(dir.join(format!("arcane-eye-vr.{index}.log"))).expect("read")
            })
            .collect();
        assert_eq!(kept, ["s0", "s1", "s2", "s3"]);
        assert!(!dir.join("arcane-eye-vr.5.log").exists());
    }

    #[test]
    fn rotation_skips_missing_sessions() {
        let folder = test_dir("rotate-gaps");
        let dir = folder.path();
        fs::write(dir.join("arcane-eye-vr.log"), "s0").expect("write");
        fs::write(dir.join("arcane-eye-vr.2.log"), "s2").expect("write");
        rotate(dir, LogName::Core).expect("rotate");
        assert_eq!(fs::read_to_string(dir.join("arcane-eye-vr.1.log")).expect("read"), "s0");
        assert!(!dir.join("arcane-eye-vr.2.log").exists());
        assert_eq!(fs::read_to_string(dir.join("arcane-eye-vr.3.log")).expect("read"), "s2");
    }

    #[test]
    fn rotation_of_a_locked_session_log_names_both_files() {
        let folder = test_dir("rotate-locked");
        let dir = folder.path();
        let base = dir.join("arcane-eye-vr.log");
        fs::write(&base, "s0").expect("write");
        let lock = fs::OpenOptions::new().read(true).share_mode(0).open(&base).expect("lock");
        let error = rotate(dir, LogName::Core).expect_err("a locked log cannot move");
        drop(lock);
        assert_eq!(error.from, base);
        assert_eq!(error.to, dir.join("arcane-eye-vr.1.log"));
    }

    #[test]
    fn rotation_onto_a_locked_oldest_log_stops_and_names_it() {
        let folder = test_dir("rotate-locked-oldest");
        let dir = folder.path();
        for index in [0, 3, 4] {
            fs::write(session_path(dir, LogName::Core, index), format!("s{index}")).expect("write");
        }
        let oldest = dir.join("arcane-eye-vr.4.log");
        let lock = fs::OpenOptions::new().read(true).share_mode(0).open(&oldest).expect("lock");
        let error = rotate(dir, LogName::Core).expect_err("a locked target cannot be replaced");
        drop(lock);
        assert_eq!(error.from, dir.join("arcane-eye-vr.3.log"));
        assert_eq!(error.to, oldest);
        assert_eq!(fs::read_to_string(dir.join("arcane-eye-vr.log")).expect("read"), "s0");
        assert_eq!(fs::read_to_string(&oldest).expect("read"), "s4");
    }

    #[test]
    fn prune_removes_only_the_oldest_fallback_logs_past_four() {
        let folder = test_dir("prune");
        let dir = folder.path();
        for process_id in 1..=6_u32 {
            let file =
                fs::File::create(fallback_path(dir, LogName::Core, process_id)).expect("create");
            let seconds = Duration::from_secs(u64::from(process_id));
            let written = SystemTime::UNIX_EPOCH.checked_add(seconds).expect("a valid time");
            file.set_modified(written).expect("set the write time");
        }
        let others = [
            "arcane-eye-vr.1.log",
            "arcane-eye-vr.pid.log",
            "arcane-eye-vr.pidx.log",
            "arcane-eye-vr-launcher.pid1.log",
        ];
        for name in others {
            fs::write(dir.join(name), "other").expect("write");
        }
        prune_fallbacks(dir, LogName::Core).expect("prune");
        let kept: Vec<bool> =
            (1..=6).map(|id| fallback_path(dir, LogName::Core, id).exists()).collect();
        assert_eq!(kept, [false, false, true, true, true, true]);
        assert!(others.iter().all(|name| dir.join(name).exists()));
    }

    #[test]
    fn launcher_log_rotates_its_own_files_and_leaves_the_core_log() {
        let folder = test_dir("rotate-launcher");
        let dir = folder.path();
        fs::write(dir.join("arcane-eye-vr.log"), "core").expect("write");
        fs::write(dir.join("arcane-eye-vr-launcher.log"), "launcher").expect("write");
        rotate(dir, LogName::Launcher).expect("rotate");
        let core = fs::read_to_string(dir.join("arcane-eye-vr.log")).expect("read");
        let launcher = fs::read_to_string(dir.join("arcane-eye-vr-launcher.1.log")).expect("read");
        let core_rotated = dir.join("arcane-eye-vr.1.log").exists();
        let fallback = fallback_path(dir, LogName::Launcher, 7);
        assert_eq!((core.as_str(), launcher.as_str()), ("core", "launcher"));
        assert!(!core_rotated, "the launcher's rotation moved the core log");
        assert_eq!(fallback, dir.join("arcane-eye-vr-launcher.pid7.log"));
    }

    #[test]
    fn reload_requests_rotate_their_own_files_and_keep_the_launch_log() {
        let folder = test_dir("rotate-reload");
        let dir = folder.path();
        fs::write(dir.join("arcane-eye-vr-launcher.log"), "launch").expect("write");
        let fallbacks: Vec<_> = (1..=5).map(|id| fallback_path(dir, LogName::Reload, id)).collect();
        for fallback in &fallbacks {
            fs::write(fallback, "fallback").expect("write");
        }
        for request in 0..6 {
            fs::write(dir.join("arcane-eye-vr-reload.log"), format!("r{request}")).expect("write");
            rotate(dir, LogName::Reload).expect("rotate");
        }
        prune_fallbacks(dir, LogName::Core).expect("prune");
        prune_fallbacks(dir, LogName::Launcher).expect("prune");
        let launch = fs::read_to_string(dir.join("arcane-eye-vr-launcher.log")).expect("read");
        let newest = fs::read_to_string(dir.join("arcane-eye-vr-reload.1.log")).expect("read");
        assert_eq!((launch.as_str(), newest.as_str()), ("launch", "r5"));
        assert!(!dir.join("arcane-eye-vr-reload.5.log").exists(), "a fifth earlier reload log");
        assert!(fallbacks.iter().all(|path| path.exists()), "another log's prune removed one");
    }
}
