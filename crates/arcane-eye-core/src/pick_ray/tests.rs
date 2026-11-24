//! The proxy decision, the cached argument and the hook's install against controllers and arguments
//! built in test memory.

use std::hint::black_box;

use diag::health::{Feature, FeatureState, Health};
use game_bindings::render_view::CameraController;
use game_bindings::state::arm_alloc_guard;
use game_mem::{GameAddr, WriteDest, try_read, try_write};
use ipc_layout::launch_token::Mode;
use settings::dev::DevFlags;

use super::{Controller, FROM_INVERSE_VIEW, PickRay, cached_argument, install};
use crate::config::Config;
use crate::eye_camera::proxy;
use crate::install::GameCode;
use crate::raw::SharedPages;
use crate::raw::test_support::{PAGE_SIZE, ReadOnlyTail, test_dest};
use crate::view_camera::test_support::{TestController, WORLD_TRANSLATE};
use crate::{logged, messages};

/// An address in the first 64 KiB, which Windows never maps.
const UNMAPPED: usize = 0x10;

/// The message of the line the first changed call logs.
const CHANGED: &str = "\"pick ray takes a proxy's ray from the eye camera's inverse view\"";

/// The message of a warning of a proxy's argument that was not set.
const NOT_SET: &str =
    "\"pick ray did not set a proxy's cached argument, so its ray follows the game camera\"";

/// A controller object whose vtable pointer is `vtable`.
struct VtableAt(Box<[usize; 1]>);

impl VtableAt {
    fn new(vtable: usize) -> Self {
        black_box(Self(Box::new([vtable])))
    }

    fn controller(&self) -> CameraController {
        CameraController::new(GameAddr::new(self.0.as_ptr().expose_provenance()))
    }
}

fn controller_of(test: &TestController) -> CameraController {
    CameraController::new(GameAddr::new(test.address()))
}

/// A cached argument of `value` at the start of `pages`.
fn argument_in(pages: &SharedPages, value: u8) -> WriteDest<'_> {
    let cached = test_dest(pages.address(), size_of::<u8>());
    try_write(cached, value).expect("a writable argument");
    cached
}

fn argument(cached: WriteDest<'_>) -> Option<u8> {
    try_read(cached.start())
}

/// The messages of the lines a fresh [`PickRay`] logs while it handles `calls` calls of `controller`
/// with the argument `cached`.
fn handled(
    name: &str,
    controller: CameraController,
    cached: WriteDest<'_>,
    calls: usize,
) -> Vec<String> {
    let pick_ray = PickRay::default();
    let ((), text) = logged(name, || {
        for _ in 0..calls {
            pick_ray.on_call(controller, cached);
        }
    });
    messages(&text)
}

#[test]
fn a_controller_is_a_proxy_only_when_slot_9_is_the_proxy_function() {
    let proxied = TestController::new(proxy::world_translate_fn());
    let other = TestController::new(WORLD_TRANSLATE);
    assert_eq!(Controller::of(controller_of(&proxied)), Controller::Proxy);
    assert_eq!(Controller::of(controller_of(&other)), Controller::Other);
}

#[test]
fn a_controller_whose_vtable_or_slot_9_cannot_be_read_is_no_proxy() {
    let unmapped_vtable = VtableAt::new(UNMAPPED);
    assert_eq!(Controller::of(unmapped_vtable.controller()), Controller::Other, "vtable");
    let pages = SharedPages::commit(PAGE_SIZE).expect("commit the test page");
    // Slot 9 of a vtable that starts at the page's last word lies past the page.
    let last_word = PAGE_SIZE.checked_sub(size_of::<usize>()).expect("a page offset");
    let vtable = pages.address().checked_add(last_word).expect("a page address");
    let short_vtable = VtableAt::new(vtable.get());
    assert_eq!(Controller::of(short_vtable.controller()), Controller::Other, "slot 9");
    let unmapped_controller = CameraController::new(GameAddr::new(UNMAPPED));
    assert_eq!(Controller::of(unmapped_controller), Controller::Other, "controller");
}

#[test]
fn a_proxy_runs_with_the_cached_argument_1_and_another_controller_with_its_own() {
    assert_eq!(FROM_INVERSE_VIEW, 1);
    assert_eq!(cached_argument(Controller::Proxy), Some(1));
    assert_eq!(cached_argument(Controller::Other), None);
}

#[test]
fn a_proxys_call_gets_the_argument_1_and_only_the_first_change_logs() {
    let proxied = TestController::new(proxy::world_translate_fn());
    let pages = SharedPages::commit(PAGE_SIZE).expect("commit the test page");
    let cached = argument_in(&pages, 0);
    let pick_ray = PickRay::default();
    let ((), text) = logged("pick-ray-first-change", || {
        pick_ray.on_call(controller_of(&proxied), cached);
        assert_eq!(argument(cached), Some(1), "the first call's argument");
        try_write(cached, 0_u8).expect("the next call's argument");
        pick_ray.on_call(controller_of(&proxied), cached);
        assert_eq!(argument(cached), Some(1), "the second call's argument");
    });
    let lines = messages(&text);
    let [line] = lines.as_slice() else { panic!("one line: {text}") };
    let controller = format!("{:#x}", proxied.address());
    assert_eq!(*line, format!("{CHANGED} controller={controller}"));
}

#[test]
fn another_controllers_call_and_a_proxys_call_with_the_argument_1_run_unchanged() {
    let other = TestController::new(WORLD_TRANSLATE);
    let proxied = TestController::new(proxy::world_translate_fn());
    let pages = SharedPages::commit(PAGE_SIZE).expect("commit the test page");
    for value in [0, 1] {
        let cached = argument_in(&pages, value);
        let lines = handled("pick-ray-other", controller_of(&other), cached, 1);
        assert_eq!(argument(cached), Some(value), "another controller's argument {value}");
        assert!(lines.is_empty(), "{lines:?}");
    }
    let cached = argument_in(&pages, 1);
    let lines = handled("pick-ray-proxy-set", controller_of(&proxied), cached, 1);
    assert_eq!(argument(cached), Some(1));
    assert!(lines.is_empty(), "an unchanged argument logs nothing: {lines:?}");
}

/// Asserts that `lines` hold one warning of a proxy's argument that was not set for `fault`.
fn assert_one_warning(lines: &[String], fault: &str) {
    let [line] = lines else { panic!("one warning: {lines:?}") };
    assert!(line.starts_with(NOT_SET) && line.ends_with(&format!("fault=\"{fault}\"")), "{line}");
}

#[test]
fn a_proxys_argument_that_cannot_be_written_logs_one_warning_for_two_calls() {
    let proxied = TestController::new(proxy::world_translate_fn());
    let tail = ReadOnlyTail::new(1, &[0]);
    let read_only = test_dest(tail.read_only_start(), size_of::<u8>());
    let lines = handled("pick-ray-read-only", controller_of(&proxied), read_only, 2);
    assert_eq!(argument(read_only), Some(0), "the call runs with its own argument");
    let address = tail.read_only_start().get();
    let fault =
        format!("a guarded write to {address:#x} faulted, so it wrote none or part of its bytes");
    assert_one_warning(&lines, &fault);
}

#[test]
fn a_proxys_argument_that_cannot_be_read_logs_one_warning_for_two_calls() {
    let proxied = TestController::new(proxy::world_translate_fn());
    let unmapped = test_dest(GameAddr::new(UNMAPPED), size_of::<u8>());
    let lines = handled("pick-ray-unmapped", controller_of(&proxied), unmapped, 2);
    assert_one_warning(&lines, &format!("the cached argument at {UNMAPPED:#x} could not be read"));
}

#[test]
fn another_controllers_unreadable_argument_logs_nothing() {
    let other = TestController::new(WORLD_TRANSLATE);
    let unmapped = test_dest(GameAddr::new(UNMAPPED), size_of::<u8>());
    let lines = handled("pick-ray-other-unmapped", controller_of(&other), unmapped, 1);
    assert!(lines.is_empty(), "{lines:?}");
}

#[test]
fn a_call_after_the_first_change_allocates_nothing() {
    let proxied = TestController::new(proxy::world_translate_fn());
    let other = TestController::new(WORLD_TRANSLATE);
    let pages = SharedPages::commit(PAGE_SIZE).expect("commit the test page");
    let cached = argument_in(&pages, 0);
    let pick_ray = PickRay::default();
    pick_ray.on_call(controller_of(&proxied), cached);
    try_write(cached, 0_u8).expect("the next call's argument");
    let guard = arm_alloc_guard().expect("arm the allocation guard");
    pick_ray.on_call(controller_of(&proxied), cached);
    pick_ray.on_call(controller_of(&other), cached);
    assert_eq!(guard.count(), 0, "the pick ray's call allocated");
    drop(guard);
    assert_eq!(argument(cached), Some(1));
}

#[test]
fn the_mode_desktop_stereo_installs_no_hook_and_logs_that() {
    let health = Health::new();
    let code = GameCode::copy();
    let config = Config::of_dev(DevFlags::default());
    let (installed, text) = logged("pick-ray-desktop", || install(&health, &code, config));
    assert!(installed.is_none(), "a hook in the mode desktop-stereo");
    assert_eq!(health.state(Feature::PickRay), FeatureState::Pending);
    let lines: Vec<&str> = text.lines().collect();
    let [line] = lines.as_slice() else { panic!("one line: {text}") };
    let end = " msg=\"pick ray hook not installed\" mode=desktop-stereo";
    assert!(line.contains(" INFO  ") && line.ends_with(end), "{line}");
}

#[test]
fn the_mode_vr_without_the_signature_installs_nothing_and_logs_the_cause() {
    let health = Health::new();
    let code = GameCode::copy();
    let config = Config { mode: Mode::Vr, ..Config::of_dev(DevFlags::default()) };
    let (installed, text) = logged("pick-ray-vr", || install(&health, &code, config));
    assert!(installed.is_none(), "a hook in the test exe");
    assert_eq!(health.state(Feature::PickRay), FeatureState::Unavailable);
    let failures: Vec<&str> =
        text.lines().filter(|line| line.contains("pick ray hook not installed")).collect();
    let [failure] = failures.as_slice() else { panic!("one failure line: {text}") };
    let cause =
        "the camera_screen_to_world_ray signature yields no address: the pattern has no match";
    assert!(
        failure.contains(" WARN  ") && failure.ends_with(&format!(" error=\"{cause}\"")),
        "{failure}"
    );
}
