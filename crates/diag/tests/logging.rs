//! Session logging end to end: events through the ring layer reach the session log file.

#[cfg(test)]
mod tests {
    use std::fmt;
    use std::fs;
    use std::os::windows::fs::OpenOptionsExt as _;
    use std::path::{Path, PathBuf};
    use std::time::{Duration, SystemTime};

    use diag::logging::{LogError, LogName};
    use tracing::{debug, info, warn};

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("arcane-eye-diag-it-{}-{name}", std::process::id()));
        if dir.exists() {
            fs::remove_dir_all(&dir).expect("remove the test folder a previous run left");
        }
        dir
    }

    /// Opens `path` with no share, so a rename, removal or second open of it fails.
    fn lock(path: &Path) -> fs::File {
        fs::OpenOptions::new().read(true).share_mode(0).open(path).expect("lock the file")
    }

    /// Writes the fallback logs of process ids 1 to 5, each older than the next.
    fn write_fallback_logs(dir: &Path) {
        for process_id in 1..=5_u64 {
            let file = fs::File::create(dir.join(format!("arcane-eye-vr.pid{process_id}.log")))
                .expect("create a fallback log");
            let seconds = Duration::from_secs(process_id);
            let written = SystemTime::UNIX_EPOCH.checked_add(seconds).expect("a valid time");
            file.set_modified(written).expect("set the write time");
        }
    }

    fn log_lines(name: &str, emit: impl FnOnce()) -> Vec<String> {
        let dir = temp_dir(name);
        let logging = diag::logging::start(&dir, LogName::Core).expect("start logging");
        tracing::dispatcher::with_default(&logging.dispatch, emit);
        let file_path = logging.handle.file_path().to_path_buf();
        logging.handle.shutdown().expect("the writer ends cleanly");
        let text = fs::read_to_string(&file_path).expect("read the session log");
        fs::remove_dir_all(&dir).expect("remove the test folder");
        text.lines().map(str::to_owned).collect()
    }

    #[test]
    fn line_holds_time_level_thread_target_and_fields() {
        let lines = log_lines("format", || info!(frame = 7, eye = "left", "eye done"));
        assert_eq!(lines.len(), 1);
        let line = lines.first().expect("one line");
        let (time, rest) = line.split_once(' ').expect("a time stamp first");
        assert_eq!(time.len(), "2026-10-30T17:59:01.123".len());
        assert_eq!(time.get(4..5), Some("-"));
        assert_eq!(time.get(10..11), Some("T"));
        assert!(rest.starts_with("INFO  thread="), "{rest}");
        assert!(rest.ends_with(" logging::tests msg=\"eye done\" frame=7 eye=left"), "{rest}");
    }

    #[test]
    fn values_with_spaces_or_quotes_are_quoted_and_escaped() {
        let lines = log_lines("quote", || warn!(path = "C:\\a b", note = "say \"hi\"", "x"));
        let line = lines.first().expect("one line");
        assert!(line.contains(" WARN  "), "{line}");
        assert!(line.ends_with(r#" msg=x path="C:\\a b" note="say \"hi\"""#), "{line}");
    }

    #[test]
    fn control_characters_equals_signs_and_empty_values_are_escaped_and_quoted() {
        let lines =
            log_lines("escape", || info!(text = "a\nb\tc\r\u{1B}", pair = "k=v", empty = "", "x"));
        assert_eq!(lines.len(), 1, "a newline in a value must not split the line");
        let line = lines.first().expect("one line");
        assert!(line.ends_with(r#" msg=x text="a\nb\tc\r\u{1B}" pair="k=v" empty="""#), "{line}");
    }

    #[test]
    fn long_quoted_value_is_cut_with_its_quote_closed() {
        let long_path = "C:\\folder".repeat(80);
        let lines = log_lines("long-value", || warn!(path = %long_path, "x"));
        let line = lines.first().expect("one line");
        let value = line.split_once(" path=\"").expect("the quoted value").1;
        let value = value.strip_suffix("[cut]\"").expect("the mark, then the closing quote");
        assert!(value.starts_with(r"C:\\folderC:\\folder"), "{line}");
        let backslashes = value.bytes().rev().take_while(|byte| *byte == b'\\').count();
        assert!(backslashes.is_multiple_of(2), "a lone backslash is left: {line}");
    }

    #[test]
    fn locked_session_log_moves_the_session_to_its_own_file_with_a_warn_line() {
        let dir = temp_dir("locked");
        fs::create_dir_all(&dir).expect("create the test folder");
        let base = dir.join("arcane-eye-vr.log");
        fs::write(&base, "previous session").expect("write the previous session log");
        let base_lock = lock(&base);
        let logging =
            diag::logging::start(&dir, LogName::Core).expect("a locked log does not stop logging");
        tracing::dispatcher::with_default(&logging.dispatch, || info!("after"));
        let file_path = logging.handle.file_path().to_path_buf();
        logging.handle.shutdown().expect("the writer ends cleanly");
        drop(base_lock);
        let text = fs::read_to_string(&file_path).expect("read the session log");
        let previous = fs::read_to_string(&base).expect("read the previous session log");
        fs::remove_dir_all(&dir).expect("remove the test folder");
        assert_eq!(file_path, dir.join(format!("arcane-eye-vr.pid{}.log", std::process::id())));
        assert_eq!(previous, "previous session");
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2, "{text}");
        let escaped = |path: PathBuf| path.display().to_string().replace('\\', "\\\\");
        let warn_fields = format!(
            " diag::logging msg=\"session log rotation failed, so this session logs to its own \
             file\" from=\"{}\" to=\"{}\" error=",
            escaped(base),
            escaped(dir.join("arcane-eye-vr.1.log"))
        );
        let warn_line = lines.first().expect("the warn line");
        assert!(warn_line.contains(" WARN  ") && warn_line.contains(&warn_fields), "{warn_line}");
        assert!(lines.get(1).expect("the info line").ends_with(" msg=after"));
    }

    #[test]
    fn reused_process_id_appends_to_its_older_fallback_log() {
        let dir = temp_dir("reused-pid");
        fs::create_dir_all(&dir).expect("create the test folder");
        let base = dir.join("arcane-eye-vr.log");
        fs::write(&base, "previous session").expect("write the previous session log");
        let fallback = dir.join(format!("arcane-eye-vr.pid{}.log", std::process::id()));
        fs::write(&fallback, "older session\n").expect("write the older fallback log");
        let base_lock = lock(&base);
        let logging =
            diag::logging::start(&dir, LogName::Core).expect("a locked log does not stop logging");
        logging.handle.shutdown().expect("the writer ends cleanly");
        drop(base_lock);
        let text = fs::read_to_string(&fallback).expect("read the fallback log");
        fs::remove_dir_all(&dir).expect("remove the test folder");
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.first(), Some(&"older session"), "{text}");
        assert!(lines.get(1).is_some_and(|line| line.contains("rotation failed")), "{text}");
    }

    #[test]
    fn second_session_in_the_same_folder_leaves_the_live_log_in_place() {
        let dir = temp_dir("live");
        let first = diag::logging::start(&dir, LogName::Core).expect("start the first session");
        let second =
            diag::logging::start(&dir, LogName::Core).expect("a live log does not stop logging");
        tracing::dispatcher::with_default(&first.dispatch, || info!("first"));
        let first_path = first.handle.file_path().to_path_buf();
        let second_path = second.handle.file_path().to_path_buf();
        first.handle.shutdown().expect("the first writer ends cleanly");
        second.handle.shutdown().expect("the second writer ends cleanly");
        let text = fs::read_to_string(&first_path).expect("read the first session log");
        let rotated = dir.join("arcane-eye-vr.1.log").exists();
        fs::remove_dir_all(&dir).expect("remove the test folder");
        assert_eq!(first_path, dir.join("arcane-eye-vr.log"));
        assert_eq!(second_path, dir.join(format!("arcane-eye-vr.pid{}.log", std::process::id())));
        assert!(!rotated, "the live log was renamed");
        assert!(text.lines().next().is_some_and(|line| line.ends_with(" msg=first")), "{text}");
    }

    #[test]
    fn a_reader_finds_the_fallback_log_the_process_wrote_or_else_the_current_log() {
        let dir = temp_dir("session-file");
        fs::create_dir_all(&dir).expect("create the test folder");
        let started = SystemTime::UNIX_EPOCH.checked_add(Duration::from_secs(10)).expect("a time");
        let earlier = SystemTime::UNIX_EPOCH.checked_add(Duration::from_secs(9)).expect("a time");
        let session_file =
            |log, process_id| diag::logging::session_file(&dir, log, process_id, started);
        let in_empty_folder = session_file(LogName::Core, 7);
        let current = fs::File::create(dir.join("arcane-eye-vr.log")).expect("create a log");
        current.set_modified(earlier).expect("set the write time");
        let beside_stale_current = session_file(LogName::Core, 7);
        current.set_modified(started).expect("set the write time");
        let beside_live_current = session_file(LogName::Core, 7);
        let other =
            fs::File::create(dir.join("arcane-eye-vr.pid8.log")).expect("create a fallback");
        other.set_modified(started).expect("set the write time");
        let beside_other_process = session_file(LogName::Core, 7);
        let own = fs::File::create(dir.join("arcane-eye-vr.pid7.log")).expect("create a fallback");
        own.set_modified(earlier).expect("set the write time");
        let beside_stale_fallback = session_file(LogName::Core, 7);
        own.set_modified(started).expect("set the write time");
        current.set_modified(earlier).expect("set the write time");
        let beside_live_fallback = session_file(LogName::Core, 7);
        let trial = fs::File::create(dir.join("arcane-eye-vr-trial.log")).expect("create a log");
        trial.set_modified(started).expect("set the write time");
        let trial_log = session_file(LogName::Trial, 7);
        drop((current, other, own, trial));
        fs::remove_dir_all(&dir).expect("remove the test folder");
        assert_eq!(in_empty_folder, None);
        assert_eq!(beside_stale_current, None);
        assert_eq!(beside_live_current, Some(dir.join("arcane-eye-vr.log")));
        assert_eq!(beside_other_process, Some(dir.join("arcane-eye-vr.log")));
        assert_eq!(beside_stale_fallback, Some(dir.join("arcane-eye-vr.log")));
        assert_eq!(beside_live_fallback, Some(dir.join("arcane-eye-vr.pid7.log")));
        assert_eq!(trial_log, Some(dir.join("arcane-eye-vr-trial.log")));
    }

    #[test]
    fn locked_stale_fallback_log_is_a_warn_line_that_names_it() {
        let dir = temp_dir("prune-locked");
        fs::create_dir_all(&dir).expect("create the test folder");
        write_fallback_logs(&dir);
        let oldest = dir.join("arcane-eye-vr.pid1.log");
        let oldest_lock = lock(&oldest);
        let logging = diag::logging::start(&dir, LogName::Core)
            .expect("a failed prune does not stop logging");
        let file_path = logging.handle.file_path().to_path_buf();
        logging.handle.shutdown().expect("the writer ends cleanly");
        drop(oldest_lock);
        let text = fs::read_to_string(&file_path).expect("read the session log");
        let oldest_stays = oldest.exists();
        fs::remove_dir_all(&dir).expect("remove the test folder");
        assert_eq!(file_path, dir.join("arcane-eye-vr.log"));
        assert!(oldest_stays);
        let path_field = format!("path=\"{}\"", oldest.display().to_string().replace('\\', "\\\\"));
        let warn_line = text.lines().next().unwrap_or_default();
        assert!(warn_line.contains(" WARN  ") && warn_line.contains("prune failed"), "{text}");
        assert!(warn_line.contains(&path_field), "{text}");
    }

    #[test]
    fn rotation_failure_is_the_first_line_when_the_prune_also_fails() {
        let dir = temp_dir("both-locked");
        fs::create_dir_all(&dir).expect("create the test folder");
        write_fallback_logs(&dir);
        let base = dir.join("arcane-eye-vr.log");
        fs::write(&base, "previous session").expect("write the previous session log");
        let base_lock = lock(&base);
        let oldest_lock = lock(&dir.join("arcane-eye-vr.pid1.log"));
        let logging = diag::logging::start(&dir, LogName::Core)
            .expect("failed start steps do not stop logging");
        let file_path = logging.handle.file_path().to_path_buf();
        logging.handle.shutdown().expect("the writer ends cleanly");
        drop((base_lock, oldest_lock));
        let text = fs::read_to_string(&file_path).expect("read the session log");
        fs::remove_dir_all(&dir).expect("remove the test folder");
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2, "{text}");
        assert!(lines.first().is_some_and(|line| line.contains("rotation failed")), "{text}");
        assert!(lines.get(1).is_some_and(|line| line.contains("prune failed")), "{text}");
    }

    #[test]
    fn locked_fallback_log_fails_with_the_fallback_path() {
        let dir = temp_dir("fallback-locked");
        fs::create_dir_all(&dir).expect("create the test folder");
        let base = dir.join("arcane-eye-vr.log");
        fs::write(&base, "previous session").expect("write the previous session log");
        let fallback = dir.join(format!("arcane-eye-vr.pid{}.log", std::process::id()));
        fs::write(&fallback, "older session\n").expect("write the older fallback log");
        let base_lock = lock(&base);
        let fallback_lock = lock(&fallback);
        let error = diag::logging::start(&dir, LogName::Core)
            .err()
            .expect("a locked fallback log stops logging");
        drop((base_lock, fallback_lock));
        fs::remove_dir_all(&dir).expect("remove the test folder");
        let LogError::Create { path: failed, .. } = &error else {
            panic!("expected a session log error: {error:?}");
        };
        assert_eq!(failed, &fallback);
    }

    struct FailingDebug;

    impl fmt::Debug for FailingDebug {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("half done")?;
            Err(fmt::Error)
        }
    }

    #[test]
    fn failing_debug_value_is_marked_cut_and_later_fields_follow() {
        let lines = log_lines("failing-debug", || info!(bad = ?FailingDebug, after = 1, "x"));
        let line = lines.first().expect("one line");
        assert!(line.ends_with(" msg=x bad=\"half done[cut]\" after=1"), "{line}");
    }

    #[test]
    fn log_folder_that_is_a_file_fails_with_the_folder_path() {
        let path =
            std::env::temp_dir().join(format!("arcane-eye-diag-it-{}-file", std::process::id()));
        fs::write(&path, "not a folder").expect("write a file where the folder goes");
        let error =
            diag::logging::start(&path, LogName::Core).err().expect("a file is not a log folder");
        fs::remove_file(&path).expect("remove the test file");
        let LogError::CreateDir { path: failed, .. } = &error else {
            panic!("expected a folder error: {error:?}");
        };
        assert_eq!(failed, &path);
    }

    #[test]
    fn debug_events_are_filtered_out_at_the_default_level() {
        let lines = log_lines("filter", || {
            debug!("hidden");
            info!("shown");
        });
        assert_eq!(lines.len(), 1);
        assert!(lines.first().expect("one line").ends_with("msg=shown"));
    }
}
