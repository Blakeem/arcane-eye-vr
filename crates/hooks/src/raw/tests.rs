//! Mid hooks installed on functions of the test binary, and the render thread cell.

use std::hint::black_box;

use diag::health::{Feature, FeatureState, Health};
use game_bindings::state::{core_context, store_core_context};
use game_mem::GameAddr;

use super::{Registers, RenderCell, install_mid_hook};
use crate::{HookContext, RenderThread};

type Target = extern "C" fn(u64, u64) -> u64;

/// The context the test thunks reach through the core context slot.
#[derive(Default)]
struct TestContext {
    health: Health,
    calls: RenderCell<u32>,
    first_argument: RenderCell<usize>,
}

impl HookContext for TestContext {
    fn health(&self) -> &Health {
        &self.health
    }
}

#[inline(never)]
extern "C" fn weighted_sum(a: u64, b: u64) -> u64 {
    a.wrapping_mul(3).wrapping_add(b)
}

#[inline(never)]
extern "C" fn exclusive_or(a: u64, b: u64) -> u64 {
    a ^ b
}

fn context() -> Option<&'static TestContext> {
    core_context::<TestContext>()
}

fn record_call(context: &TestContext, thread: &RenderThread, registers: &mut Registers) {
    context.calls.with(thread, |calls| *calls = calls.saturating_add(1));
    context.first_argument.with(thread, |argument| *argument = registers.rcx());
}

fn record_call_then_panic(context: &TestContext, thread: &RenderThread, registers: &mut Registers) {
    record_call(context, thread, registers);
    panic!("the handler panicked");
}

crate::render_mid_hook!(recording_thunk, Feature::PresentHook, context(), record_call);
crate::render_mid_hook!(panicking_thunk, Feature::PresentHook, context(), record_call_then_panic);

fn address(target: Target) -> GameAddr {
    GameAddr::new((target as *const ()).expose_provenance())
}

/// Calls `target` through an opaque pointer, so the compiler cannot inline or fold the call.
fn call(target: Target, a: u64, b: u64) -> u64 {
    black_box(target)(black_box(a), black_box(b))
}

fn calls(context: &TestContext) -> u32 {
    context.calls.with(&RenderThread::new(), |calls| *calls).expect("the cell is free")
}

/// One test runs both hooks in turn, since they share the process's one core context slot and the
/// panic latches its feature off for the rest of the process.
#[test]
fn mid_hook_runs_its_handler_and_a_caught_panic_latches_the_feature_off() {
    store_core_context(TestContext::default()).expect("the slot is empty");
    let context = context().expect("the test context");

    // SAFETY: `weighted_sum` is a function of this binary, and the hook sits on its first instruction.
    let recording =
        unsafe { install_mid_hook(address(weighted_sum), recording_thunk) }.expect("install");
    assert_eq!(call(weighted_sum, 2, 5), 11, "a pending feature's hook runs no handler");
    assert_eq!(calls(context), 0);
    context.health.install_ok(Feature::PresentHook);
    assert_eq!(call(weighted_sum, 2, 5), 11, "the hook leaves the result unchanged");
    assert_eq!(calls(context), 1);
    let argument = context.first_argument.with(&RenderThread::new(), |argument| *argument);
    assert_eq!(argument, Some(2), "the handler sees the target's rcx");
    drop(recording);
    assert_eq!(call(weighted_sum, 2, 5), 11);
    assert_eq!(calls(context), 1, "a removed hook runs no handler");

    // SAFETY: `exclusive_or` is a function of this binary, and the hook sits on its first instruction.
    let panicking =
        unsafe { install_mid_hook(address(exclusive_or), panicking_thunk) }.expect("install");
    assert_eq!(call(exclusive_or, 6, 3), 5, "the target runs after a caught panic");
    assert_eq!(calls(context), 2);
    assert_eq!(context.health.state(Feature::PresentHook), FeatureState::Failed);
    assert_eq!(call(exclusive_or, 6, 3), 5);
    assert_eq!(calls(context), 2, "a failed feature's hook runs no handler");
    drop(panicking);
}

#[test]
fn render_cell_refuses_a_nested_lend_and_lends_again_after_a_panic() {
    let thread = RenderThread::new();
    let cell = RenderCell::new(1_u32);
    let nested = cell.with(&thread, |value| {
        *value = 2;
        cell.with(&thread, |_| ())
    });
    assert_eq!(nested, Some(None), "a nested lend is refused");
    let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        cell.with(&thread, |_| panic!("the lend panicked"))
    }));
    drop(unwound.expect_err("the lend's panic reaches catch_unwind"));
    assert_eq!(cell.with(&thread, |value| *value), Some(2), "the cell lends again after a panic");
}

#[test]
fn install_codes_decode_to_their_errors() {
    use super::{InlineHookError, decode};
    use crate::HookError;
    assert_eq!(decode(0), Ok(()));
    assert_eq!(decode(1), Err(HookError::Allocation));
    assert_eq!(decode(2), Err(HookError::Exception));
    assert_eq!(decode(16), Err(HookError::InlineHook(InlineHookError::BadAllocation)));
    assert_eq!(decode(22), Err(HookError::InlineHook(InlineHookError::NotEnoughSpace)));
    assert_eq!(decode(23), Err(HookError::UnknownCode(23)));
    assert_eq!(decode(3), Err(HookError::UnknownCode(3)));
    assert_eq!(decode(-1), Err(HookError::UnknownCode(-1)));
}
