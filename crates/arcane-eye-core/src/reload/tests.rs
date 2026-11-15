//! The reload step against files in a test folder, and the poll thread's start and stop.

use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use diag::logging::{self, LogName};
use settings::dev::DevFlags;
use settings::player::{IpdMm, Settings};
use settings::watch::Watch;
use tracing::dispatcher;

use super::{Published, start_with, step};
use crate::test_dir;

fn with_ipd(millimeters: f32) -> Settings {
    let mut settings = Settings::default();
    settings.stereo.ipd_mm = IpdMm::try_from(millimeters).expect("in range");
    settings
}

/// A watch of `root` after its first check, as the core's start leaves it.
fn checked(root: &Path) -> Watch {
    let mut watch = Watch::new(root);
    drop(watch.check());
    watch
}

/// Runs one step and returns the messages it logged.
fn step_logged(watch: &mut Watch, published: &Published, log_dir: &Path) -> Vec<String> {
    let logging = logging::start(log_dir, LogName::Core).expect("start logging");
    dispatcher::with_default(&logging.dispatch, || step(watch, DevFlags::default(), published));
    let file_path = logging.handle.file_path().to_path_buf();
    logging.handle.shutdown().expect("the writer ends cleanly");
    let text = fs::read_to_string(file_path).expect("read the session log");
    let messages = text
        .lines()
        .map(|line| line.split_once(" msg=").map_or(line, |(_, message)| message).to_owned());
    messages.collect()
}

#[test]
fn a_changed_player_file_becomes_the_published_settings() {
    let dir = test_dir("reload-changed");
    let root = dir.path();
    let mut watch = checked(root);
    let published = Published::new(Settings::default());
    assert_eq!(step_logged(&mut watch, &published, &root.join("logs-1")), [""; 0]);
    assert_eq!(published.get(), Settings::default());
    fs::write(root.join("settings.toml"), "[stereo]\nipd_mm = 72.5\nipd = 1\n").expect("write");
    let lines = step_logged(&mut watch, &published, &root.join("logs-2"));
    assert_eq!(published.get(), with_ipd(72.5));
    let expected = [
        "\"unknown settings key ignored\" file=settings.toml key=stereo.ipd",
        "\"settings reloaded\" ipd_mm=72.5",
    ];
    assert_eq!(lines, expected);
}

#[test]
fn a_rejected_player_file_keeps_the_settings_before_and_logs_the_rejection() {
    let dir = test_dir("reload-rejected");
    let root = dir.path();
    let mut watch = checked(root);
    let published = Published::new(with_ipd(58.0));
    fs::write(root.join("settings.toml"), "[stereo]\nipd_mm = 500\n").expect("write");
    let lines = step_logged(&mut watch, &published, &root.join("logs"));
    assert_eq!(published.get(), with_ipd(58.0));
    let [line] = lines.as_slice() else { panic!("one warning: {lines:?}") };
    let message = "\"changed settings file gave no values, so the settings before stay\"";
    assert!(line.starts_with(message), "{line}");
    assert!(line.contains("ipd_mm must be from 50 to 80 millimeters, got 500"), "{line}");
}

#[test]
fn a_changed_dev_file_changes_no_setting_and_logs_that_its_flags_wait_for_the_next_start() {
    let dir = test_dir("reload-dev");
    let root = dir.path();
    let mut watch = checked(root);
    let published = Published::new(Settings::default());
    fs::write(root.join("dev.toml"), "[stereo]\nsecond_view = true\n").expect("write");
    let lines = step_logged(&mut watch, &published, &root.join("logs-1"));
    assert_eq!(published.get(), Settings::default());
    let expected = "\"dev file changed, and its flags apply at the next start\" \
                    main_view_eye=center second_view=true";
    assert_eq!(lines, [expected]);
    fs::write(root.join("dev.toml"), "").expect("write");
    let back = step_logged(&mut watch, &published, &root.join("logs-2"));
    assert_eq!(back, [""; 0], "the flags read at start again");
}

#[test]
fn the_poll_thread_publishes_a_change_and_ends_when_its_handle_drops() {
    let dir = test_dir("reload-thread");
    let root = dir.path();
    let published = Arc::new(Published::new(Settings::default()));
    let period = Duration::from_millis(5);
    let reload = start_with(checked(root), DevFlags::default(), Arc::clone(&published), period)
        .expect("start the poll thread");
    fs::write(root.join("settings.toml"), "[stereo]\nipd_mm = 61\n").expect("write");
    let deadline = Instant::now() + Duration::from_secs(10);
    while published.get() != with_ipd(61.0) && Instant::now() < deadline {
        thread::park_timeout(period);
    }
    assert_eq!(published.get(), with_ipd(61.0));
    let super::Reload { _stop: stop, _thread: thread } = reload;
    drop(stop);
    thread.join().expect("the poll thread ends without a panic");
}
