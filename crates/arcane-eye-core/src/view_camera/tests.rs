//! The main view choice and the camera read against views and controllers built in test memory.

use std::fs;

use diag::health::{Feature, FeatureState, Health};
use diag::logging::LogName;
use game_bindings::camera::PerCamera;
use game_mem::{GameAddr, try_read, try_write};
use stereo_core::eye::Eye;

use super::test_support::{TestController, TestView, WORLD_TRANSLATE, block_at};
use super::{ViewCamera, Viewpoint, install};
use crate::eye_camera::proxy;
use crate::eye_camera::test_support::{HEAD_POSITION, HeadController, eye_state};
use crate::install::GameCode;
use crate::raw::SharedPages;
use crate::raw::test_support::PAGE_SIZE;
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
    camera.end_frame(&running()).map(|chosen| chosen.camera.position[0])
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
    let mut camera = ViewCamera::new(Some(WORLD_TRANSLATE), None);
    assert_eq!(frame_camera_x(&mut camera, &[&special, &main]), Some(2.0));
}

#[test]
fn a_view_whose_controller_has_another_slot_9_function_is_skipped() {
    let other = TestController::new(OTHER_FUNCTION);
    let gameplay = TestController::new(WORLD_TRANSLATE);
    let other_view = TestView::new(other.address(), &block_at(1.0));
    let main = TestView::new(gameplay.address(), &block_at(2.0));
    let mut camera = ViewCamera::new(Some(WORLD_TRANSLATE), None);
    assert_eq!(frame_camera_x(&mut camera, &[&other_view, &main]), Some(2.0));
}

#[test]
fn the_first_matching_view_of_a_frame_wins_and_a_second_is_ignored() {
    let controller = TestController::new(WORLD_TRANSLATE);
    let first = TestView::new(controller.address(), &block_at(1.0));
    let second = TestView::new(controller.address(), &block_at(2.0));
    let mut camera = ViewCamera::new(Some(WORLD_TRANSLATE), None);
    assert_eq!(frame_camera_x(&mut camera, &[&first, &second]), Some(1.0));
}

#[test]
fn the_choice_starts_again_after_a_frame_end() {
    let controller = TestController::new(WORLD_TRANSLATE);
    let first = TestView::new(controller.address(), &block_at(1.0));
    let second = TestView::new(controller.address(), &block_at(2.0));
    let mut camera = ViewCamera::new(Some(WORLD_TRANSLATE), None);
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
    let mut camera = ViewCamera::new(Some(WORLD_TRANSLATE), None);
    assert_eq!(frame_camera_x(&mut camera, &[&main, &later]), None);
}

#[test]
fn no_view_is_the_main_view_without_a_resolved_function() {
    // A controller whose vtable slot 9 is null against an unresolved function.
    let controller = TestController::new(GameAddr::new(0));
    let view = TestView::new(controller.address(), &block_at(1.0));
    let mut camera = ViewCamera::new(None, None);
    assert_eq!(frame_camera_x(&mut camera, &[&view]), None);
}

#[test]
fn a_feature_latched_off_ends_the_frame_with_no_camera_and_a_fresh_choice() {
    let controller = TestController::new(WORLD_TRANSLATE);
    let first = TestView::new(controller.address(), &block_at(1.0));
    let second = TestView::new(controller.address(), &block_at(2.0));
    let mut camera = ViewCamera::new(Some(WORLD_TRANSLATE), None);
    let health = running();
    camera.on_view(first.view());
    health.panic_caught(Feature::ViewCamera);
    assert_eq!(camera.end_frame(&health), None, "a camera chosen before the latch");
    assert_eq!(frame_camera_x(&mut camera, &[&second]), Some(2.0), "the choice started again");
}

#[test]
fn a_head_view_renders_from_the_center_with_its_own_position_as_the_head() {
    let controller = TestController::new(WORLD_TRANSLATE);
    let main = TestView::new(controller.address(), &block_at(2.0));
    let mut camera = ViewCamera::new(Some(WORLD_TRANSLATE), Some(Eye::Left));
    camera.on_view(main.view());
    let frame = camera.end_frame(&running()).expect("a head camera");
    assert_eq!(frame.viewpoint, Viewpoint::Head);
    assert_eq!(frame.head(), [2.0, 0.0, 0.0], "the head is the view's own PerCamera position");
}

#[test]
fn a_view_on_an_eye_proxy_renders_from_that_eye_with_the_proxied_head() {
    let head = HeadController::new(WORLD_TRANSLATE);
    let main = TestView::new(head.address().get(), &block_at(2.0));
    let mut eye_camera = eye_state(Eye::Left);
    eye_camera.on_view_begin(main.view());
    let mut camera = ViewCamera::new(Some(WORLD_TRANSLATE), Some(Eye::Left));
    camera.on_view(main.view());
    let frame = camera.end_frame(&running()).expect("an eye camera");
    let Viewpoint::Eye { eye, head: head_position } = frame.viewpoint else {
        panic!("an eye viewpoint: {frame:?}")
    };
    assert_eq!(eye, Eye::Left);
    assert_eq!(frame.camera.position, [2.0, 0.0, 0.0], "the view's own PerCamera block");
    for (axis, (read, measured)) in head_position.iter().zip(HEAD_POSITION).enumerate() {
        assert!((read - measured).abs() <= 1e-3, "head axis {axis}: {read} vs {measured}");
    }
}

#[test]
fn a_proxied_main_view_whose_head_position_is_unreadable_ends_its_frame_with_no_camera() {
    let template = TestController::new(proxy::world_translate_fn());
    let vtable = try_read::<usize>(GameAddr::new(template.address())).expect("the vtable pointer");
    // Windows reserves address space in 64 KiB units, so the bytes past this one page up to the next
    // 64 KiB boundary belong to no allocation, and the proxy's head position lies there.
    let pages = SharedPages::commit(PAGE_SIZE).expect("commit the test page");
    let last_word = PAGE_SIZE.checked_sub(size_of::<usize>()).expect("a page offset");
    let controller = pages.address().checked_add(last_word).expect("a test address");
    try_write(controller, vtable).expect("a writable controller");
    let proxied = TestView::new(controller.get(), &block_at(1.0));
    let head = TestController::new(WORLD_TRANSLATE);
    let later = TestView::new(head.address(), &block_at(2.0));
    let mut camera = ViewCamera::new(Some(WORLD_TRANSLATE), Some(Eye::Left));
    assert_eq!(proxy::head_position(controller), None, "the head position past the page");
    assert_eq!(frame_camera_x(&mut camera, &[&proxied, &later]), None);
}

#[test]
fn a_controller_with_the_proxy_function_is_no_main_view_without_the_eye_camera() {
    let controller = TestController::new(proxy::world_translate_fn());
    let view = TestView::new(controller.address(), &block_at(1.0));
    let mut camera = ViewCamera::new(Some(WORLD_TRANSLATE), None);
    assert_eq!(frame_camera_x(&mut camera, &[&view]), None);
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
