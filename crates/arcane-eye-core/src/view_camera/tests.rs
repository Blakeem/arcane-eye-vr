//! The main view choice and the camera read against views and controllers built in test memory.

use std::fs;

use diag::health::{Feature, FeatureState, Health};
use diag::logging::LogName;
use game_bindings::camera::PerCamera;
use game_mem::GameAddr;

use super::test_support::{TestController, TestView, WORLD_TRANSLATE, block_at};
use super::{ViewCamera, install};
use crate::install::GameCode;
use crate::test_dir;

/// A slot 9 function of another controller type. No code runs there.
const OTHER_FUNCTION: GameAddr = GameAddr::new(0x7FF6_0000_2000);

/// A registry with the view camera running.
fn running() -> Health {
    let health = Health::new();
    health.install_ok(Feature::ViewCamera);
    health
}

/// The X position of the camera that `views` give the frame's main view, in the order they pass.
fn frame_camera_x(camera: &mut ViewCamera, views: &[&TestView]) -> Option<f32> {
    for view in views {
        camera.on_view(view.view());
    }
    camera.end_frame(&running()).map(|chosen| chosen.position[0])
}

/// The lines of the session log that `install` writes, and whether it installed a hook.
fn install_log(health: &Health, name: &str) -> (bool, String) {
    let dir = test_dir(name);
    let logging = diag::logging::start(dir.path(), LogName::Core).expect("start logging");
    let installed = tracing::dispatcher::with_default(&logging.dispatch, || {
        install(health, &GameCode::copy()).is_some()
    });
    let file_path = logging.handle.file_path().to_path_buf();
    logging.handle.shutdown().expect("the writer ends cleanly");
    let text = fs::read_to_string(&file_path).expect("read the session log");
    (installed, text)
}

/// The one failure line of `text`, which must end with ` error="<cause>"`.
fn assert_one_failure(text: &str, cause: &str) {
    let failures: Vec<&str> =
        text.lines().filter(|line| line.contains("view camera hook not installed")).collect();
    let [failure] = failures.as_slice() else { panic!("one failure line: {text}") };
    let expected_end = format!(" error=\"{cause}\"");
    assert!(failure.contains(" WARN  ") && failure.ends_with(&expected_end), "{failure}");
}

#[test]
fn a_view_with_a_null_controller_is_skipped() {
    let controller = TestController::new(WORLD_TRANSLATE);
    let special = TestView::new(0, &block_at(1.0));
    let main = TestView::new(controller.address(), &block_at(2.0));
    let mut camera = ViewCamera::new(Some(WORLD_TRANSLATE));
    assert_eq!(frame_camera_x(&mut camera, &[&special, &main]), Some(2.0));
}

#[test]
fn a_view_whose_controller_has_another_slot_9_function_is_skipped() {
    let other = TestController::new(OTHER_FUNCTION);
    let gameplay = TestController::new(WORLD_TRANSLATE);
    let other_view = TestView::new(other.address(), &block_at(1.0));
    let main = TestView::new(gameplay.address(), &block_at(2.0));
    let mut camera = ViewCamera::new(Some(WORLD_TRANSLATE));
    assert_eq!(frame_camera_x(&mut camera, &[&other_view, &main]), Some(2.0));
}

#[test]
fn the_first_matching_view_of_a_frame_wins_and_a_second_is_ignored() {
    let controller = TestController::new(WORLD_TRANSLATE);
    let first = TestView::new(controller.address(), &block_at(1.0));
    let second = TestView::new(controller.address(), &block_at(2.0));
    let mut camera = ViewCamera::new(Some(WORLD_TRANSLATE));
    assert_eq!(frame_camera_x(&mut camera, &[&first, &second]), Some(1.0));
}

#[test]
fn the_choice_starts_again_after_a_frame_end() {
    let controller = TestController::new(WORLD_TRANSLATE);
    let first = TestView::new(controller.address(), &block_at(1.0));
    let second = TestView::new(controller.address(), &block_at(2.0));
    let mut camera = ViewCamera::new(Some(WORLD_TRANSLATE));
    assert_eq!(frame_camera_x(&mut camera, &[&first]), Some(1.0));
    assert_eq!(frame_camera_x(&mut camera, &[&second, &first]), Some(2.0));
    assert_eq!(frame_camera_x(&mut camera, &[]), None, "a frame with no view has no camera");
}

#[test]
fn a_main_view_whose_block_does_not_decode_still_ends_the_choice_of_its_frame() {
    let controller = TestController::new(WORLD_TRANSLATE);
    let flat = PerCamera { projection: [0.0; 16], ..block_at(1.0) };
    let main = TestView::new(controller.address(), &flat);
    let later = TestView::new(controller.address(), &block_at(2.0));
    let mut camera = ViewCamera::new(Some(WORLD_TRANSLATE));
    assert_eq!(frame_camera_x(&mut camera, &[&main, &later]), None);
}

#[test]
fn no_view_is_the_main_view_without_a_resolved_function() {
    // A controller whose vtable slot 9 is null against an unresolved function.
    let controller = TestController::new(GameAddr::new(0));
    let view = TestView::new(controller.address(), &block_at(1.0));
    let mut camera = ViewCamera::new(None);
    assert_eq!(frame_camera_x(&mut camera, &[&view]), None);
}

#[test]
fn a_feature_latched_off_ends_the_frame_with_no_camera_and_a_fresh_choice() {
    let controller = TestController::new(WORLD_TRANSLATE);
    let first = TestView::new(controller.address(), &block_at(1.0));
    let second = TestView::new(controller.address(), &block_at(2.0));
    let mut camera = ViewCamera::new(Some(WORLD_TRANSLATE));
    let health = running();
    camera.on_view(first.view());
    health.panic_caught(Feature::ViewCamera);
    assert_eq!(camera.end_frame(&health), None, "a camera chosen before the latch");
    assert_eq!(frame_camera_x(&mut camera, &[&second]), Some(2.0), "the choice started again");
}

#[test]
fn install_in_an_exe_without_the_signatures_installs_nothing_and_logs_the_cause() {
    let health = Health::new();
    health.install_ok(Feature::PresentHook);
    let (installed, text) = install_log(&health, "view-camera-install");
    assert!(!installed, "a hook in the test exe");
    assert_eq!(health.state(Feature::ViewCamera), FeatureState::Unavailable);
    assert!(health.is_running(Feature::PresentHook), "the view camera latched the Present hook");
    let cause = "the get_world_translate signature yields no address: the pattern has no match";
    assert_one_failure(&text, cause);
}

#[test]
fn install_without_a_running_present_hook_installs_nothing_and_logs_the_cause() {
    let health = Health::new();
    health.install_failed(Feature::PresentHook);
    let (installed, text) = install_log(&health, "view-camera-no-present");
    assert!(!installed, "a hook with no frame end");
    assert_eq!(health.state(Feature::ViewCamera), FeatureState::Unavailable);
    let cause =
        "the Present call-site hook, which ends each frame of the view camera, is not running";
    assert_one_failure(&text, cause);
}
