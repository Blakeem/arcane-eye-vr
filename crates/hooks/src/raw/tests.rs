//! Mid hooks installed on functions of the test binary, and the render thread cell.

use std::hint::black_box;

use diag::health::{Feature, FeatureState, Health};
use game_bindings::state::{core_context, store_core_context};
use game_mem::GameAddr;

use super::{Registers, RenderCell, install_mid_hook};
use crate::{GameThread, HookContext, RenderThread, Runs};

type Target = extern "C" fn(u64, u64) -> u64;

/// The context the test thunks reach through the core context slot.
#[derive(Default)]
struct TestContext {
    health: Health,
    calls: RenderCell<u32>,
    game_calls: RenderCell<u32>,
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

#[inline(never)]
extern "C" fn difference(a: u64, b: u64) -> u64 {
    a.wrapping_sub(b)
}

/// Counts a game thread call and keeps its `rcx`, then panics when `rcx` is [`PANIC_ARGUMENT`].
fn record_game_call(context: &TestContext, _thread: &GameThread, registers: &mut Registers) {
    let cells = RenderThread::new();
    context.game_calls.with(&cells, |calls| *calls = calls.saturating_add(1));
    context.first_argument.with(&cells, |argument| *argument = registers.rcx());
    assert_ne!(registers.rcx(), PANIC_ARGUMENT, "the game thread handler panicked");
}

/// The first argument that makes the game thread handler panic.
const PANIC_ARGUMENT: usize = 99;

crate::render_mid_hook!(
    recording_thunk,
    Feature::PresentHook,
    Runs::WhileActive,
    context(),
    record_call
);
crate::render_mid_hook!(
    panicking_thunk,
    Feature::PresentHook,
    Runs::WhileActive,
    context(),
    record_call_then_panic
);
crate::render_mid_hook!(
    render_after_failure_thunk,
    Feature::PresentHook,
    Runs::AfterFailureToo,
    context(),
    record_call
);
crate::game_mid_hook!(game_thunk, Feature::EyeView, Runs::WhileActive, context(), record_game_call);
crate::game_mid_hook!(
    after_failure_thunk,
    Feature::EyeView,
    Runs::AfterFailureToo,
    context(),
    record_game_call
);

#[inline(never)]
extern "C" fn bitwise_or(a: u64, b: u64) -> u64 {
    a | b
}

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

fn game_calls(context: &TestContext) -> u32 {
    context.game_calls.with(&RenderThread::new(), |calls| *calls).expect("the cell is free")
}

/// One test runs every hook in turn, since they share the process's one core context slot and a
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

    // SAFETY: `weighted_sum` is a function of this binary, and the hook sits on its first instruction.
    let after_failure =
        unsafe { install_mid_hook(address(weighted_sum), render_after_failure_thunk) }
            .expect("install");
    assert_eq!(call(weighted_sum, 4, 1), 13, "the hook leaves the result unchanged");
    assert_eq!(calls(context), 3, "the render hook's handler runs on a failed feature");
    drop(after_failure);

    game_hook_runs_its_handler_and_a_caught_panic_latches_its_feature_off(context);
}

/// The game thread hook's part of the test above, with a feature of its own.
fn game_hook_runs_its_handler_and_a_caught_panic_latches_its_feature_off(context: &TestContext) {
    // SAFETY: `difference` is a function of this binary, and the hook sits on its first instruction.
    let game = unsafe { install_mid_hook(address(difference), game_thunk) }.expect("install");
    assert_eq!(call(difference, 9, 4), 5, "a pending feature's game hook runs no handler");
    assert_eq!(game_calls(context), 0);
    context.health.install_ok(Feature::EyeView);
    assert_eq!(call(difference, 9, 4), 5, "the game hook leaves the result unchanged");
    assert_eq!(game_calls(context), 1);
    let argument = context.first_argument.with(&RenderThread::new(), |argument| *argument);
    assert_eq!(argument, Some(9), "the game thread handler sees the target's rcx");
    assert_eq!(call(difference, 99, 4), 95, "the target runs after a caught panic");
    assert_eq!(game_calls(context), 2);
    assert_eq!(context.health.state(Feature::EyeView), FeatureState::Failed);
    assert_eq!(call(difference, 9, 4), 5);
    assert_eq!(game_calls(context), 2, "a failed feature's game hook runs no handler");
    drop(game);

    game_hook_after_failure_runs_its_handler_on_a_failed_feature(context);
}

/// A game thread hook that also runs after its feature failed, on the feature the test above
/// latched off.
fn game_hook_after_failure_runs_its_handler_on_a_failed_feature(context: &TestContext) {
    // SAFETY: `bitwise_or` is a function of this binary, and the hook sits on its first instruction.
    let hook =
        unsafe { install_mid_hook(address(bitwise_or), after_failure_thunk) }.expect("install");
    assert_eq!(call(bitwise_or, 8, 1), 9, "the hook leaves the result unchanged");
    assert_eq!(game_calls(context), 3, "the handler runs on a failed feature");
    assert_eq!(call(bitwise_or, 99, 4), 103, "the target runs after a caught panic");
    assert_eq!(game_calls(context), 4);
    assert_eq!(context.health.state(Feature::EyeView), FeatureState::Failed, "Failed stays");
    assert_eq!(call(bitwise_or, 8, 1), 9);
    assert_eq!(game_calls(context), 5, "the handler runs after its own caught panic");
    drop(hook);
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
