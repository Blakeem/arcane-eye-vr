//! The eye view feature's decision, retry and install. The create and the game update run against a
//! world in test memory in the submodules.

mod create;
mod update;

use std::fs;
use std::time::{Duration, Instant};

use diag::health::{Feature, FeatureState, Health};
use diag::logging::LogName;
use game_bindings::render_view::{Environment, ViewContent};
use game_mem::GameAddr;
use settings::dev::{DevFlags, MainViewEye, SecondView, StereoFlags};
use settings::player::Settings;

use super::decide::{Action, Observed, RETRY_PERIOD, Readiness, Retry, Stale, decide, readiness};
use super::{guarded, install};
use crate::config::Config;
use crate::install::GameCode;
use crate::test_dir;

fn logged<R>(name: &str, run: impl FnOnce() -> R) -> (R, String) {
    let dir = test_dir(name);
    let logging = diag::logging::start(dir.path(), LogName::Core).expect("start logging");
    let result = tracing::dispatcher::with_default(&logging.dispatch, run);
    let file_path = logging.handle.file_path().to_path_buf();
    logging.handle.shutdown().expect("the writer ends cleanly");
    (result, fs::read_to_string(&file_path).expect("read the session log"))
}

fn config(second_view: SecondView) -> Config {
    let stereo = StereoFlags { main_view_eye: MainViewEye::Center, second_view };
    Config { settings: Settings::default(), dev: DevFlags { stereo } }
}

/// A record whose address, id and request id all derive from `id`.
fn environment(id: i32) -> Environment {
    let offset = usize::try_from(id).expect("a test id");
    let record = GameAddr::new(0x5000).checked_add(offset).expect("a test address");
    let request = id.checked_add(100).expect("a test request id");
    Environment { record, id: Some(id), request }
}

/// A main view's content with every value set.
fn set_main() -> ViewContent {
    ViewContent {
        controller: GameAddr::new(0x1000),
        scene: GameAddr::new(0x2000),
        scene_active: 1,
        atmosphere: environment(11),
        lighting: environment(22),
        split_index: 0,
        atmosphere_lock: 0,
        lighting_lock: 0,
    }
}

#[test]
fn the_present_call_creates_only_without_an_eye_view_and_when_the_retry_is_due() {
    assert_eq!(decide(Observed::PresentWithoutEye, true), Action::Create);
    assert_eq!(decide(Observed::PresentWithoutEye, false), Action::Nothing);
    assert_eq!(decide(Observed::PresentWithEye, true), Action::Nothing);
    assert_eq!(decide(Observed::PresentWithEye, false), Action::Nothing);
}

#[test]
fn a_game_update_without_an_eye_view_or_of_another_pipeline_does_nothing() {
    for due in [true, false] {
        assert_eq!(decide(Observed::UpdateWithoutEye, due), Action::Nothing);
        assert_eq!(decide(Observed::UpdateOfOtherPipeline, due), Action::Nothing);
    }
}

#[test]
fn every_stale_reason_forgets_the_eye_view() {
    let reasons = [
        Stale::PipelineChanged,
        Stale::MainViewChanged,
        Stale::FrameChanged,
        Stale::NotListed,
        Stale::NotGameRenderView,
    ];
    for reason in reasons {
        for due in [true, false] {
            assert_eq!(decide(Observed::Stale(reason), due), Action::Forget, "{reason:?}");
        }
    }
}

#[test]
fn a_live_eye_view_is_placed_by_its_readiness() {
    let live = Observed::Live;
    assert_eq!(decide(live(Readiness::Ready), false), Action::SideBySide);
    assert_eq!(decide(live(Readiness::Ready), true), Action::SideBySide);
    assert_eq!(decide(live(Readiness::NeedsSetUp), true), Action::SetUp);
    assert_eq!(decide(live(Readiness::NeedsSetUp), false), Action::FullFrame);
    assert_eq!(decide(live(Readiness::MainUnset), true), Action::FullFrame);
    assert_eq!(decide(live(Readiness::MainUnset), false), Action::FullFrame);
}

#[test]
fn a_retry_is_due_at_first_and_again_one_second_after_an_attempt() {
    let start = Instant::now();
    let mut retry = Retry::default();
    assert!(retry.due(start));
    retry.wait(start);
    assert!(!retry.due(start));
    assert!(!retry.due(start + Duration::from_millis(999)));
    assert!(retry.due(start + RETRY_PERIOD));
    assert_eq!(RETRY_PERIOD, Duration::from_secs(1));
}

#[test]
fn a_main_view_without_a_controller_scene_or_environment_is_unset() {
    let eye = set_main();
    let missing = Environment { record: GameAddr::new(0), id: None, request: -1 };
    let unsets: [fn(&mut ViewContent, Environment); 5] = [
        |main, _| main.controller = GameAddr::new(0),
        |main, _| main.scene = GameAddr::new(0),
        |main, _| main.scene_active = 0,
        |main, none| main.atmosphere = none,
        |main, none| main.lighting = none,
    ];
    for unset in unsets {
        let mut main = set_main();
        unset(&mut main, missing);
        assert_eq!(readiness(&main, &eye), Readiness::MainUnset, "{main:?}");
    }
}

#[test]
fn an_eye_view_is_ready_only_when_its_four_values_equal_the_main_views() {
    let main = set_main();
    assert_eq!(readiness(&main, &main), Readiness::Ready);
    let mut fresh = main;
    fresh.controller = GameAddr::new(0);
    fresh.scene = GameAddr::new(0);
    assert_eq!(readiness(&main, &fresh), Readiness::NeedsSetUp);
    let mut other_atmosphere = main;
    other_atmosphere.atmosphere = environment(12);
    assert_eq!(readiness(&main, &other_atmosphere), Readiness::NeedsSetUp);
    let mut other_lighting = main;
    other_lighting.lighting = environment(23);
    assert_eq!(readiness(&main, &other_lighting), Readiness::NeedsSetUp);
    let mut same_ids = main;
    same_ids.atmosphere.record = GameAddr::new(0x9000);
    same_ids.split_index = 1;
    assert_eq!(readiness(&main, &same_ids), Readiness::Ready, "records and guards do not count");
}

#[test]
fn a_panic_at_the_present_call_latches_only_the_eye_view_off() {
    let health = Health::new();
    health.install_ok(Feature::PresentHook);
    let mut runs = 0;
    guarded(&health, || runs += 1);
    assert_eq!(runs, 0, "a pending feature runs nothing");
    health.install_ok(Feature::EyeView);
    guarded(&health, || runs += 1);
    assert_eq!(runs, 1);
    guarded(&health, || panic!("the eye view's present work panicked"));
    assert_eq!(health.state(Feature::EyeView), FeatureState::Failed);
    assert!(health.is_running(Feature::PresentHook), "the Present call-site hook keeps running");
    guarded(&health, || runs += 1);
    assert_eq!(runs, 1, "a failed feature runs nothing");
}

#[test]
fn second_view_off_installs_no_hook_and_logs_that_once() {
    let health = Health::new();
    health.install_ok(Feature::PresentHook);
    let code = GameCode::copy();
    let (installed, text) =
        logged("eye-view-off", || install(&health, &code, config(SecondView::Off)));
    assert!(installed.is_none(), "a hook with second_view off");
    assert_eq!(health.state(Feature::EyeView), FeatureState::Pending);
    let lines: Vec<&str> = text.lines().collect();
    let [line] = lines.as_slice() else { panic!("one line: {text}") };
    assert!(line.contains(" INFO  "), "{line}");
    assert!(line.ends_with(" msg=\"eye view hook not installed\" second_view=false"), "{line}");
}

/// The one line of `text` that names the failed install, which must end with ` error="<cause>"`.
fn assert_one_failure(text: &str, cause: &str) {
    let failures: Vec<&str> =
        text.lines().filter(|line| line.contains("eye view hook not installed")).collect();
    let [failure] = failures.as_slice() else { panic!("one failure line: {text}") };
    let expected_end = format!(" error=\"{cause}\"");
    assert!(failure.contains(" WARN  ") && failure.ends_with(&expected_end), "{failure}");
}

#[test]
fn second_view_on_without_a_running_present_hook_installs_nothing_and_logs_the_cause() {
    let health = Health::new();
    health.install_failed(Feature::PresentHook);
    let code = GameCode::copy();
    let (installed, text) =
        logged("eye-view-no-present", || install(&health, &code, config(SecondView::On)));
    assert!(installed.is_none(), "a hook with no Present call-site hook");
    assert_eq!(health.state(Feature::EyeView), FeatureState::Unavailable);
    assert_one_failure(
        &text,
        "the Present call-site hook, which creates the eye view, is not running",
    );
}

#[test]
fn second_view_on_in_an_exe_without_the_signatures_installs_nothing_and_logs_the_cause() {
    let health = Health::new();
    health.install_ok(Feature::PresentHook);
    let code = GameCode::copy();
    let (installed, text) =
        logged("eye-view-install", || install(&health, &code, config(SecondView::On)));
    assert!(installed.is_none(), "a hook in the test exe");
    assert_eq!(health.state(Feature::EyeView), FeatureState::Unavailable);
    assert_one_failure(
        &text,
        "the g_pipeline signature yields no address: the pattern has no match",
    );
}
