//! The init sequence against test roots, test mappings and test event names.

use super::{InitError, InitStatus, Names, guarded, open_status, run, start, start_reload};
use crate::config::Config;
use crate::context::CoreContext;
use crate::raw::reload_event::{self, Event};
use crate::raw::test_support::TestMapping;
use crate::reload::Published;
use crate::{logged, test_dir};
use diag::health::{Feature, FeatureState};
use diag::logging::LogError;
use diag::root::{Root, RootError};
use game_bindings::state::{self, StateError};
use ipc_layout::launch_token::{Mode, Status, UnknownMode};
use settings::dev::DevFlags;
use settings::player::Settings;
use std::fs;
use std::sync::Arc;
use tracing::{Dispatch, dispatcher};

#[test]
fn a_reload_event_that_already_exists_starts_no_settings_thread_and_logs_the_cause() {
    let dir = test_dir("reload-taken");
    let name = reload_event::test_name("reload-taken");
    let taken = Event::named(&name).expect("create the event first");
    let published = Arc::new(Published::new(Settings::default()));
    let (reload, log) = logged("reload-taken-log", || {
        start_reload(dir.path(), &name, Config::of_dev(DevFlags::default()), &published)
    });
    assert!(reload.is_none(), "a settings thread on a taken event");
    let line = log.lines().next().expect("the warning");
    assert!(line.contains(" WARN  "), "{line}");
    let escaped = name.replace('\\', "\\\\");
    let expected = format!(
        " msg=\"settings reload not started, so a changed settings file applies at the next \
         start\" error=\"the settings thread's events were not created: the event {escaped} \
         already exists, so another process in this session holds it\""
    );
    assert!(line.ends_with(&expected), "{line}");
    drop(taken);
}

#[test]
fn start_writes_the_start_line_to_the_session_log() {
    let dir = test_dir("start");
    let core = start(Mode::DesktopStereo, dir.path()).expect("start");
    let file_path = core.log.file_path().to_path_buf();
    core.log.shutdown().expect("the writer ends cleanly");
    let text = fs::read_to_string(&file_path).expect("read the session log");
    let line = text.lines().find(|line| line.contains("msg=\"core started\"")).expect("start line");
    assert!(line.contains(" INFO  "), "{line}");
    assert!(line.contains(&format!(" version={} ", env!("CARGO_PKG_VERSION"))), "{line}");
    assert!(line.contains(" mode=desktop-stereo "), "{line}");
    let escaped_path = file_path.display().to_string().replace('\\', "\\\\");
    assert!(line.ends_with(&format!(" log_file=\"{escaped_path}\"")), "{line}");
    assert_eq!(file_path, dir.path().join("arcane-eye-vr.log"));
}

/// The only test that sets the process-wide subscriber and fills the slot, since each takes one
/// value per process. The root folder holds no dev file, so the second view is on and each
/// feature of the pair is asked to install. The test exe holds no game signature, so the run
/// reports status 12 and installs no hook.
#[test]
fn second_run_returns_already_running_and_a_taken_subscriber_maps_to_code_3() {
    let folder = test_dir("second-run");
    let dir = folder.path().to_path_buf();
    let mapping = TestMapping::create("second-run");
    let reload_event = reload_event::test_name("second-run");
    let names = Names { token: &mapping.name, reload_event: &reload_event };
    run(1, || Ok(Root::new(dir.clone())), names).expect("the first run starts the core");
    assert_eq!(mapping.status(), Status::SignatureFailure as u32);
    let context = state::core_context::<CoreContext>().expect("the stored context");
    assert_eq!(context.health.state(Feature::PresentHook), FeatureState::Unavailable);
    assert_eq!(context.health.state(Feature::ViewCamera), FeatureState::Unavailable);
    assert_eq!(context.health.state(Feature::GameState), FeatureState::Unavailable);
    for feature in [
        Feature::EyeCamera,
        Feature::EyeView,
        Feature::ShadowCache,
        Feature::ObjectSlot,
        Feature::UiLayer,
    ] {
        assert_eq!(
            context.health.state(feature),
            FeatureState::Unavailable,
            "{feature:?} was not asked to install, or installed with no Present call-site hook"
        );
    }
    reload_event::signal(&reload_event).expect("the settings thread holds the reload event");
    assert!(dir.join(r"logs\arcane-eye-vr.log").is_file(), "the session log is in root\\logs");
    let second = run(1, || Ok(Root::new(dir.clone())), names);
    assert!(matches!(second, Err(InitError::AlreadyRunning)));
    assert!(
        !dir.join(r"logs\arcane-eye-vr.1.log").exists(),
        "a second start would have rotated the log"
    );
    let taken = dispatcher::set_global_default(Dispatch::none())
        .expect_err("the first run set the process-wide subscriber");
    assert_eq!(guarded(move || Err(InitError::from(taken))), 3);
}

#[test]
fn an_unknown_mode_fails_with_code_6_and_starts_nothing() {
    let folder = test_dir("unknown-mode");
    let dir = folder.path().join("root");
    let mapping = TestMapping::create("unknown-mode");
    let reload_event = reload_event::test_name("unknown-mode");
    let names = Names { token: &mapping.name, reload_event: &reload_event };
    let result = run(7, || Ok(Root::new(dir.clone())), names);
    assert!(matches!(result, Err(InitError::Mode(UnknownMode(7)))), "{result:?}");
    assert!(!dir.exists(), "an unknown mode started session logging");
    assert_eq!(mapping.status(), Status::Waiting as u32, "an unknown mode wrote a status");
    assert!(reload_event::signal(&reload_event).is_err(), "an unknown mode created the event");
    assert_eq!(guarded(|| Err(InitError::Mode(UnknownMode(7)))), 6);
    let zero = run(0, || Ok(Root::new(dir.clone())), names);
    assert!(matches!(zero, Err(InitError::Mode(UnknownMode(0)))));
}

#[test]
fn failed_init_logs_its_code_and_cause_through_the_current_subscriber() {
    let (code, text) = logged("failure-line", || guarded(|| Err(InitError::AlreadyRunning)));
    assert_eq!(code, 1);
    let line = text.lines().next().expect("the failure line");
    assert!(line.contains(" WARN  "), "{line}");
    assert!(
        line.ends_with(" msg=\"arcane_eye_init failed\" code=1 error=AlreadyRunning"),
        "{line}"
    );
}

#[test]
fn a_token_that_fails_to_open_reports_no_status_and_logs_its_cause() {
    let (status, text) =
        logged("status-open-failure", || open_status(r"NoSuchNamespace\arcane-eye-vr-launch"));
    assert!(status.is_none(), "a token that failed to open");
    let line = text.lines().next().expect("the failure line");
    assert!(line.contains(" WARN  "), "{line}");
    let cause = " msg=\"launch token status is not reported\" error=\"opening the launch \
                 token's mapping failed: ";
    assert!(line.contains(cause), "{line}");
}

#[test]
fn guarded_maps_success_errors_and_panics_to_codes() {
    assert_eq!(guarded(|| Ok(())), 0);
    assert_eq!(guarded(|| Err(InitError::AlreadyRunning)), 1);
    assert_eq!(guarded(|| Err(InitError::Log(LogError::WriterPanicked))), 2);
    assert_eq!(guarded(|| Err(InitError::State(StateError::CoreContextFull))), 4);
    assert_eq!(guarded(|| panic!("init panicked")), InitStatus::Panicked as u32);
    assert_eq!(InitStatus::Panicked as u32, 5);
    let root = RootError::LocalAppData(windows::core::Error::empty());
    assert_eq!(guarded(|| Err(InitError::Root(root))), 7);
}
