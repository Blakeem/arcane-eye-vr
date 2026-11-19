//! The reload step against files in a test folder, and the settings thread's start, request and stop.

use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use ipc_layout::launch_token::Mode;
use settings::dev::{DevFlags, SecondView};
use settings::player::{DepthOffsetPercent, IpdMm, Settings};
use tracing::{Dispatch, dispatcher};

use super::{Published, Reload, StartError, apply, run, start};
use crate::config::Config;
use crate::raw::reload_event::{self, Event, EventError};
use crate::{logged, messages, test_dir};

/// Long enough for the settings thread to apply a request on a loaded test machine.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// Longer than one second, so a thread that reads the files on a timer of a second or less reads
/// them within it.
const QUIET_TIME: Duration = Duration::from_millis(1500);

fn with_ipd(millimeters: f32) -> Settings {
    let mut settings = Settings::default();
    settings.stereo.ipd_mm = IpdMm::try_from(millimeters).expect("in range");
    settings
}

/// The config of a start with the default files in the mode `desktop-stereo`.
fn default_start() -> Config {
    Config::of_dev(DevFlags::default())
}

/// Runs one reload of the files in `root` in the mode `desktop-stereo` and returns the messages it
/// logged under the session log of test folder `log_name`.
fn apply_logged(root: &Path, published: &Published, log_name: &str) -> Vec<String> {
    apply_logged_in(Mode::DesktopStereo, root, published, log_name)
}

/// [`apply_logged`] after a start in the launch mode `mode`.
fn apply_logged_in(mode: Mode, root: &Path, published: &Published, log_name: &str) -> Vec<String> {
    let started = Config { mode, ..default_start() };
    let ((), text) = logged(log_name, || apply(root, started, published));
    messages(&text)
}

/// Waits until `published` holds `expected` or the request timeout passes.
fn wait_for(published: &Published, expected: Settings) {
    let deadline = Instant::now().checked_add(REQUEST_TIMEOUT).expect("a deadline in range");
    while published.get() != expected && Instant::now() < deadline {
        thread::park_timeout(Duration::from_millis(5));
    }
}

/// Waits the quiet time out, so a thread that reads the files on its own has read them by its end.
fn wait_quietly() {
    let deadline = Instant::now().checked_add(QUIET_TIME).expect("a deadline in range");
    while Instant::now() < deadline {
        thread::park_timeout(deadline.saturating_duration_since(Instant::now()));
    }
}

/// Signals the stop event and waits for the thread to end.
fn stop(reload: Reload) {
    let Reload { _stop: stop, _thread: thread } = reload;
    drop(stop);
    thread.join().expect("the settings thread ends without a panic");
}

#[test]
fn a_player_file_that_parses_becomes_the_published_settings_and_logs_its_values() {
    let dir = test_dir("reload-parsed");
    let root = dir.path();
    let published = Published::new(with_ipd(58.0));
    let player = "[stereo]\nipd_mm = 72.5\ndepth_offset_percent = 4\nipd = 1\n";
    fs::write(root.join("settings.toml"), player).expect("write");
    let mut changed = with_ipd(72.5);
    changed.stereo.depth_offset_percent = DepthOffsetPercent::try_from(4.0).expect("in range");
    let lines = apply_logged(root, &published, "reload-parsed-log-1");
    assert_eq!(published.get(), changed);
    let expected = [
        "\"unknown settings key ignored\" file=settings.toml key=stereo.ipd",
        "\"settings reloaded\" ipd_mm=72.5 depth_offset_percent=4.0",
    ];
    assert_eq!(lines, expected);
    let again = apply_logged(root, &published, "reload-parsed-log-2");
    assert_eq!(published.get(), changed);
    assert_eq!(again, expected, "an unchanged file is read and logged again");
}

#[test]
fn a_missing_player_file_publishes_the_defaults() {
    let dir = test_dir("reload-missing");
    let root = dir.path();
    let published = Published::new(with_ipd(58.0));
    let lines = apply_logged(root, &published, "reload-missing-log");
    assert_eq!(published.get(), Settings::default());
    assert_eq!(lines, ["\"settings reloaded\" ipd_mm=65.0 depth_offset_percent=1.5"]);
}

#[test]
fn a_rejected_player_file_keeps_the_settings_in_use_and_logs_the_rejection() {
    let dir = test_dir("reload-rejected");
    let root = dir.path();
    let published = Published::new(with_ipd(58.0));
    fs::write(root.join("settings.toml"), "[stereo]\nipd_mm = 500\n").expect("write");
    let lines = apply_logged(root, &published, "reload-rejected-log");
    assert_eq!(published.get(), with_ipd(58.0));
    let [line] = lines.as_slice() else { panic!("one warning: {lines:?}") };
    let message = "\"settings file gave no values, so the settings in use stay\"";
    assert!(line.starts_with(message), "{line}");
    assert!(line.contains("ipd_mm must be from 50 to 80 millimeters, got 500"), "{line}");
}

#[test]
fn dev_flags_that_differ_from_the_start_change_no_setting_and_wait_for_the_next_start() {
    let dir = test_dir("reload-dev");
    let root = dir.path();
    let published = Published::new(Settings::default());
    fs::write(root.join("dev.toml"), "[stereo]\nsecond_view = false\n").expect("write");
    let lines = apply_logged(root, &published, "reload-dev-log-1");
    assert_eq!(published.get(), Settings::default());
    let expected = [
        "\"settings reloaded\" ipd_mm=65.0 depth_offset_percent=1.5",
        "\"dev file changed, and its flags apply at the next start\" main_view_eye=center \
         second_view=false viewer_source=back_buffer",
    ];
    assert_eq!(lines, expected);
    assert_eq!(DevFlags::default().stereo.second_view, SecondView::On);
    fs::write(root.join("dev.toml"), "[stereo]\nmain_view_eye = \"up\"\n").expect("write");
    let rejected = apply_logged(root, &published, "reload-dev-log-2");
    let [_, warning] = rejected.as_slice() else { panic!("two lines: {rejected:?}") };
    let message = "\"dev file gave no values, and the flags read at start stay\"";
    assert!(warning.starts_with(message), "{warning}");
    fs::write(root.join("dev.toml"), "").expect("write");
    let back = apply_logged(root, &published, "reload-dev-log-3");
    assert_eq!(
        back,
        ["\"settings reloaded\" ipd_mm=65.0 depth_offset_percent=1.5"],
        "the flags read at start again"
    );
}

#[test]
fn a_changed_vr_table_logs_its_own_line_in_the_mode_vr_only() {
    let dir = test_dir("reload-dev-vr");
    let root = dir.path();
    let published = Published::new(Settings::default());
    fs::write(root.join("dev.toml"), "[vr]\ntest_head = \"sweep\"\n").expect("write");
    let reloaded = "\"settings reloaded\" ipd_mm=65.0 depth_offset_percent=1.5";
    let desktop = apply_logged(root, &published, "reload-dev-vr-log-1");
    assert_eq!(desktop, [reloaded], "the mode desktop-stereo never reads the [vr] table");
    let vr = apply_logged_in(Mode::Vr, root, &published, "reload-dev-vr-log-2");
    let vr_changed = "\"dev file changed, and its vr flags apply at the next start\" \
                      horizon=level test_head=sweep test_yaw_deg=20.0";
    assert_eq!(vr, [reloaded, vr_changed]);
    fs::write(root.join("dev.toml"), "[stereo]\nsecond_view = false\n").expect("write");
    let stereo = apply_logged_in(Mode::Vr, root, &published, "reload-dev-vr-log-3");
    let changed = "\"dev file changed, and its flags apply at the next start\" main_view_eye=center \
                   second_view=false viewer_source=back_buffer";
    assert_eq!(stereo, [reloaded, changed], "an unchanged [vr] table logs no vr line");
}

#[test]
fn the_started_thread_publishes_a_changed_file_on_a_request_and_ends_when_its_handle_drops() {
    let dir = test_dir("reload-thread");
    let root = dir.path();
    let name = reload_event::test_name("thread");
    let published = Arc::new(Published::new(Settings::default()));
    let folder = root.to_path_buf();
    let reload = start(folder, &name, default_start(), Arc::clone(&published))
        .expect("start the settings thread");
    fs::write(root.join("settings.toml"), "[stereo]\nipd_mm = 61\n").expect("write");
    wait_quietly();
    assert_eq!(published.get(), Settings::default(), "a file read with no request");
    reload_event::signal(&name).expect("signal the reload event");
    wait_for(&published, with_ipd(61.0));
    assert_eq!(published.get(), with_ipd(61.0));
    stop(reload);
    let gone = reload_event::signal(&name);
    assert!(gone.is_err(), "the reload event outlived the ended thread");
}

#[test]
fn the_thread_reads_the_files_once_per_request_and_never_without_one() {
    let dir = test_dir("reload-run");
    let root = dir.path();
    let name = reload_event::test_name("run");
    let reload = Event::named(&name).expect("create the reload event");
    let stop = Arc::new(Event::unnamed().expect("create the stop event"));
    let published = Arc::new(Published::new(Settings::default()));
    let (stopped, shared, folder) = (Arc::clone(&stop), Arc::clone(&published), root.to_path_buf());
    let ((), text) = logged("reload-run-log", || {
        let dispatch = dispatcher::get_default(Dispatch::clone);
        let thread = thread::spawn(move || {
            let run_thread = || run(&stopped, &reload, &folder, default_start(), &shared);
            dispatcher::with_default(&dispatch, run_thread);
        });
        fs::write(root.join("settings.toml"), "[stereo]\nipd_mm = 61\n").expect("write");
        wait_quietly();
        assert_eq!(published.get(), Settings::default(), "a file read with no request");
        reload_event::signal(&name).expect("signal the reload event");
        wait_for(&published, with_ipd(61.0));
        fs::write(root.join("settings.toml"), "[stereo]\nipd_mm = 63\n").expect("write");
        reload_event::signal(&name).expect("signal the reload event");
        wait_for(&published, with_ipd(63.0));
        stop.set().expect("signal the stop event");
        thread.join().expect("the settings thread ends without a panic");
    });
    assert_eq!(published.get(), with_ipd(63.0));
    let reloads = [
        "\"settings reloaded\" ipd_mm=61.0 depth_offset_percent=1.5",
        "\"settings reloaded\" ipd_mm=63.0 depth_offset_percent=1.5",
    ];
    assert_eq!(messages(&text), reloads, "one read for each request and none without one");
}

#[test]
fn a_reload_event_name_that_another_kind_of_object_holds_starts_no_thread() {
    let dir = test_dir("reload-mutex");
    let name = reload_event::test_name("mutex");
    let mutex = reload_event::hold_with_mutex(&name);
    let published = Arc::new(Published::new(Settings::default()));
    let started = start(dir.path().to_path_buf(), &name, default_start(), published);
    assert!(matches!(started, Err(StartError::Event(EventError::Create(_)))), "{started:?}");
    drop(mutex);
}

#[test]
fn a_reload_event_that_exists_starts_no_second_thread() {
    let dir = test_dir("reload-exists");
    let name = reload_event::test_name("exists");
    let published = Arc::new(Published::new(Settings::default()));
    let folder = dir.path().to_path_buf();
    let first = start(folder.clone(), &name, default_start(), Arc::clone(&published))
        .expect("start the settings thread");
    let second = start(folder, &name, default_start(), published);
    assert!(
        matches!(second, Err(StartError::Event(EventError::Exists { name: ref named })) if *named == name),
        "{second:?}"
    );
    stop(first);
}
