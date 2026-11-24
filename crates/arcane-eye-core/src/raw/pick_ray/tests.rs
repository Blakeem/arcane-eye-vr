//! The pick ray handler's path through a mid hook on a function of the test binary that takes the
//! ray function's arguments.

use std::hint::black_box;

use game_mem::GameAddr;
use hooks::Registers;

use super::ray_call;
use crate::eye_camera::proxy;
use crate::pick_ray::PickRay;
use crate::view_camera::test_support::{TestController, WORLD_TRANSLATE};

type Ray = extern "C" fn(usize, f32, f32, usize, usize, u8) -> u8;

/// A function with the ray function's arguments that returns the cached argument it ran with.
#[inline(never)]
extern "C" fn ray(controller: usize, u: f32, v: f32, near: usize, far: usize, cached: u8) -> u8 {
    black_box((controller, u, v, near, far));
    cached
}

/// The handler's path with a fresh feature state, since a thunk reaches no test state. A panic
/// here would abort the test process, and no step of the path panics.
extern "C" fn ray_thunk(registers: &mut Registers) {
    let Some((controller, cached)) = ray_call(registers) else { return };
    PickRay::default().on_call(controller, cached);
}

/// Calls `ray` through an opaque pointer, so the compiler cannot inline or fold the call.
fn call(controller: usize, cached: u8) -> u8 {
    let function: Ray = ray;
    black_box(function)(black_box(controller), 0.25, 0.75, 0, 0, black_box(cached))
}

#[test]
fn the_hook_gives_a_proxys_call_the_argument_1_and_leaves_another_controllers_call() {
    let proxied = TestController::new(proxy::world_translate_fn());
    let head = TestController::new(WORLD_TRANSLATE);
    let function: Ray = ray;
    let address = GameAddr::new((function as *const ()).expose_provenance());
    // SAFETY: `ray` is a function of this binary, and the hook sits on its first instruction.
    let hook = unsafe { hooks::install_mid_hook(address, ray_thunk) }.expect("install");
    assert_eq!(call(proxied.address(), 0), 1, "a proxy's call");
    assert_eq!(call(proxied.address(), 1), 1, "a proxy's call with the argument 1");
    assert_eq!(call(head.address(), 0), 0, "the head controller's call");
    assert_eq!(call(head.address(), 1), 1, "the head controller's call with the argument 1");
    drop(hook);
    assert_eq!(call(proxied.address(), 0), 0, "a removed hook changes nothing");
}
