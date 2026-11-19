//! The eye camera's proxy of the main view against views, controllers and vtables built in test
//! memory. The submodules hold the stereo pair and the install.

mod install;
mod pair;

use game_bindings::offsets::{
    CONTROLLER_INVERSE_VIEW, CONTROLLER_SIZE, CONTROLLER_VIEW, CONTROLLER_VTABLE_SLOTS,
    CONTROLLER_WORLD_TRANSLATE_SLOT, VIEW_LAST_SEEN_CONTROLLER, VIEW_READ_STATE_CONTROLLER,
};
use game_bindings::render_view::{CameraController, RenderView};
use game_bindings::state::arm_alloc_guard;
use game_mem::{GameAddr, WriteError, try_read, try_read_array, try_read_bytes, try_write};
use settings::player::{IpdMm, StereoSettings};
use stereo_core::eye::{Eye, EyeScale, PerEye};

use super::lane::last_seen_fault;
use super::proxy::{self, PROXY_SIZE, RING_LEN};
use super::test_support::{HALF_IPD_M, HEAD_POSITION, HEAD_VIEW, HeadController, RIGHT, eye_state};
use super::{EyeFault, EyeState, History, Outcome, offsets_m};
use crate::eye_view::Layout;
use crate::logged;
use crate::raw::SharedPages;
use crate::raw::test_support::{PAGE_SIZE, ReadOnlyTail, call_world_translate, view_writes};
use crate::view_camera::test_support::{TestView, WORLD_TRANSLATE, block_at};

/// A slot 9 function of another controller type. No code runs there.
const OTHER_FUNCTION: GameAddr = GameAddr::new(0x7FF6_0000_2000);

/// Windows reserves the lowest 64 KiB of every process, so no read or write there succeeds.
const UNMAPPED: GameAddr = GameAddr::new(0x10);

/// Three x64 Windows pages, which hold a view's fields up to its `ReadState`.
const VIEW_PAGES_LEN: usize = 3 * PAGE_SIZE;

/// The rotation elements of a view matrix.
const ROTATION: [usize; 9] = [0, 1, 2, 4, 5, 6, 8, 9, 10];

fn assert_near(actual: &[f32], expected: &[f32], what: &str) {
    assert_eq!(actual.len(), expected.len(), "{what}");
    for (index, (a, e)) in actual.iter().zip(expected).enumerate() {
        assert!((a - e).abs() <= 1e-3, "{what} element {index}: {a} vs {e}");
    }
}

/// The measured head position moved `offset_m` along the measured right axis.
fn eye_position(offset_m: f32) -> [f32; 3] {
    [0, 1, 2].map(|axis| HEAD_POSITION[axis] + RIGHT[axis] * offset_m)
}

fn at(base: GameAddr, offset: usize) -> GameAddr {
    base.checked_add(offset).expect("a test address")
}

fn floats<const N: usize>(address: GameAddr) -> [f32; N] {
    try_read_array(address).expect("readable floats")
}

/// What the eye camera does with `view` at its begin-recording.
fn begin(camera: &mut EyeState, view: RenderView) -> Result<Outcome, EyeFault> {
    camera.proxy_view(view, view_writes(view))
}

/// The proxy that `outcome` names, after checking that it is a proxied outcome with `history`.
fn proxied(outcome: Result<Outcome, EyeFault>, history: History) -> GameAddr {
    match outcome {
        Ok(Outcome::Proxied { proxy, history: got, .. }) if got == history => proxy,
        other => panic!("a proxied view with {history:?}: {other:?}"),
    }
}

/// A view at the address whose `ReadState` field is the last pointer before `end` and holds `head`.
fn view_ending_before(end: GameAddr, head: &HeadController) -> RenderView {
    let before = size_of::<usize>().checked_add(VIEW_READ_STATE_CONTROLLER).expect("an offset");
    let view_address = end.get().checked_sub(before).expect("a test address");
    let view = RenderView::new(GameAddr::new(view_address));
    let read_state = view_writes(view).offset(VIEW_READ_STATE_CONTROLLER).expect("the field");
    try_write(read_state, head.address().get()).expect("a writable ReadState");
    view
}

#[test]
fn the_proxy_slot_9_called_through_its_vtable_returns_the_eye_position() {
    let head = HeadController::new(WORLD_TRANSLATE);
    let view = TestView::new(head.address().get(), &block_at(0.0));
    let mut camera = eye_state(Eye::Right);
    let proxy = proxied(begin(&mut camera, view.view()), History::Cut);
    assert_eq!(view.pointer_at(VIEW_READ_STATE_CONTROLLER), proxy.get());
    let vtable = GameAddr::new(try_read::<usize>(proxy).expect("the proxy's vtable pointer"));
    let slot_9 = try_read::<usize>(at(vtable, CONTROLLER_WORLD_TRANSLATE_SLOT)).map(GameAddr::new);
    assert_eq!(slot_9, Some(proxy::world_translate_fn()), "the proxy's slot 9");
    let translate = call_world_translate(proxy::world_translate_fn(), proxy);
    assert_near(&floats::<3>(translate), &eye_position(HALF_IPD_M), "the right eye position");
}

#[test]
fn new_offsets_move_the_eye_of_the_next_frames_proxy_and_keep_the_views_history() {
    let head = HeadController::new(WORLD_TRANSLATE);
    let view = TestView::new(head.address().get(), &block_at(0.0));
    let mut camera = eye_state(Eye::Right);
    proxied(begin(&mut camera, view.view()), History::Cut);
    view.finish_frame(head.address().get());
    camera.set_offsets(PerEye { left: -0.04, right: 0.04 });
    camera.end_frame(Layout::MainOnly);
    let proxy = proxied(begin(&mut camera, view.view()), History::Kept);
    let translate = call_world_translate(proxy::world_translate_fn(), proxy);
    assert_near(&floats::<3>(translate), &eye_position(0.04), "the right eye at the new offset");
}

#[test]
fn the_eye_offsets_are_the_full_eye_distance_times_the_eye_scale_in_use() {
    let ipd_mm = IpdMm::try_from(64.0).expect("an eye distance in range");
    let stereo = StereoSettings { ipd_mm, ..StereoSettings::default() };

    assert_eq!(offsets_m(stereo, EyeScale::FULL), PerEye { left: -0.032, right: 0.032 });
    assert_eq!(offsets_m(stereo, EyeScale::clamped(0.5)), PerEye { left: -0.016, right: 0.016 });
    assert_eq!(offsets_m(stereo, EyeScale::clamped(0.0)), PerEye { left: 0.0, right: 0.0 });
}

#[test]
fn the_proxy_holds_the_eye_matrices_and_else_the_head_controllers_bytes_and_slots() {
    let head = HeadController::new(WORLD_TRANSLATE);
    let view = TestView::new(head.address().get(), &block_at(0.0));
    let mut camera = eye_state(Eye::Left);
    let proxy = proxied(begin(&mut camera, view.view()), History::Cut);
    let bytes = try_read_bytes(proxy, CONTROLLER_SIZE).expect("the proxy's bytes");
    assert_eq!(bytes[8..CONTROLLER_VIEW], head.bytes()[8..CONTROLLER_VIEW]);
    let after_inverse = CONTROLLER_INVERSE_VIEW.checked_add(64).expect("an offset");
    assert_eq!(bytes[after_inverse..], head.bytes()[after_inverse..]);
    let eye_view = floats::<16>(at(proxy, CONTROLLER_VIEW));
    assert_eq!(ROTATION.map(|i| eye_view[i].to_bits()), ROTATION.map(|i| HEAD_VIEW[i].to_bits()));
    let moved_x = HEAD_VIEW[12] + HALF_IPD_M;
    assert_eq!(eye_view[12].to_bits(), moved_x.to_bits(), "the left eye's camera-space x");
    assert_eq!(eye_view[13..].to_vec(), HEAD_VIEW[13..].to_vec());
    let inverse = floats::<16>(at(proxy, CONTROLLER_INVERSE_VIEW));
    for (transposed, element) in [(1, 4), (2, 8), (6, 9), (4, 1), (8, 2), (9, 6), (0, 0)] {
        assert_eq!(inverse[transposed], eye_view[element], "inverse element {transposed}");
    }
    assert_near(&inverse[12..15], &eye_position(-HALF_IPD_M), "the inverse's translation");
    let vtable = GameAddr::new(try_read::<usize>(proxy).expect("the proxy's vtable pointer"));
    let slots = try_read_array::<usize, CONTROLLER_VTABLE_SLOTS>(vtable).expect("the slots");
    let slot_9 = CONTROLLER_WORLD_TRANSLATE_SLOT.checked_div(size_of::<usize>()).expect("a slot");
    for (index, (copied, original)) in slots.iter().zip(head.vtable()).enumerate() {
        if index != slot_9 {
            assert_eq!(copied, original, "vtable slot {index}");
        }
    }
    let head_position = proxy::head_position(proxy).expect("the proxy's head position");
    assert_near(&head_position, &HEAD_POSITION, "the head position");
}

#[test]
fn the_proxy_is_the_last_seen_controller_only_while_the_head_controller_stays() {
    let head = HeadController::new(WORLD_TRANSLATE);
    let other_head = HeadController::new(WORLD_TRANSLATE);
    let view = TestView::new(head.address().get(), &block_at(0.0));
    let mut camera = eye_state(Eye::Right);
    proxied(begin(&mut camera, view.view()), History::Cut);
    assert_eq!(view.pointer_at(VIEW_LAST_SEEN_CONTROLLER), 0, "the first frame keeps the field");
    view.finish_frame(head.address().get());
    camera.end_frame(Layout::MainOnly);
    let second = proxied(begin(&mut camera, view.view()), History::Kept);
    assert_eq!(view.pointer_at(VIEW_LAST_SEEN_CONTROLLER), second.get());
    view.finish_frame(other_head.address().get());
    camera.end_frame(Layout::MainOnly);
    let third = proxied(begin(&mut camera, view.view()), History::Cut);
    assert_eq!(view.pointer_at(VIEW_READ_STATE_CONTROLLER), third.get());
    assert_eq!(view.pointer_at(VIEW_LAST_SEEN_CONTROLLER), second.get(), "a changed controller");
}

#[test]
fn a_frame_on_another_controller_between_two_proxied_frames_is_a_camera_cut() {
    let head = HeadController::new(WORLD_TRANSLATE);
    let other = HeadController::new(OTHER_FUNCTION);
    let view = TestView::new(head.address().get(), &block_at(0.0));
    let mut camera = eye_state(Eye::Left);
    let first = proxied(begin(&mut camera, view.view()), History::Cut);
    view.finish_frame(other.address().get());
    camera.end_frame(Layout::MainOnly);
    assert_eq!(begin(&mut camera, view.view()), Ok(Outcome::Unchanged));
    view.finish_frame(head.address().get());
    camera.end_frame(Layout::MainOnly);
    assert_eq!(view.pointer_at(VIEW_LAST_SEEN_CONTROLLER), other.address().get());
    let third = proxied(begin(&mut camera, view.view()), History::Cut);
    assert_eq!(view.pointer_at(VIEW_LAST_SEEN_CONTROLLER), other.address().get(), "a real cut");
    assert_eq!(first.checked_add(PROXY_SIZE), Some(third), "a frame on another takes no proxy");
}

#[test]
fn only_the_first_main_view_of_a_frame_is_proxied_until_the_frame_ends() {
    let head = HeadController::new(WORLD_TRANSLATE);
    let first = TestView::new(head.address().get(), &block_at(0.0));
    let second = TestView::new(head.address().get(), &block_at(0.0));
    let mut camera = eye_state(Eye::Right);
    proxied(begin(&mut camera, first.view()), History::Cut);
    assert_eq!(begin(&mut camera, second.view()), Ok(Outcome::Unchanged));
    assert_eq!(second.pointer_at(VIEW_READ_STATE_CONTROLLER), head.address().get());
    camera.end_frame(Layout::MainOnly);
    proxied(begin(&mut camera, second.view()), History::Cut);
}

#[test]
fn the_ring_takes_each_of_its_proxies_once_then_starts_again() {
    let head = HeadController::new(WORLD_TRANSLATE);
    let view = TestView::new(head.address().get(), &block_at(0.0));
    let mut camera = eye_state(Eye::Left);
    let mut proxies = vec![proxied(begin(&mut camera, view.view()), History::Cut).get()];
    for _ in 0..RING_LEN {
        view.finish_frame(head.address().get());
        camera.end_frame(Layout::MainOnly);
        proxies.push(proxied(begin(&mut camera, view.view()), History::Kept).get());
    }
    let first = proxies[0];
    for (frame, proxy) in proxies.iter().enumerate().take(RING_LEN) {
        let offset = frame.checked_mul(PROXY_SIZE).and_then(|bytes| first.checked_add(bytes));
        assert_eq!(Some(*proxy), offset, "frame {frame}");
    }
    assert_eq!(proxies[RING_LEN], first, "the ninth frame reuses the first proxy");
}

#[test]
fn a_view_with_a_null_or_another_controller_is_left_as_it_is() {
    let other = HeadController::new(OTHER_FUNCTION);
    let special = TestView::new(0, &block_at(0.0));
    let other_view = TestView::new(other.address().get(), &block_at(0.0));
    let mut camera = eye_state(Eye::Right);
    assert_eq!(begin(&mut camera, special.view()), Ok(Outcome::Unchanged));
    assert_eq!(begin(&mut camera, other_view.view()), Ok(Outcome::Unchanged));
    assert_eq!(special.pointer_at(VIEW_READ_STATE_CONTROLLER), 0);
    assert_eq!(other_view.pointer_at(VIEW_READ_STATE_CONTROLLER), other.address().get());
}

#[test]
fn an_unreadable_view_or_controller_is_a_fault_with_one_warning_per_period() {
    let mut camera = eye_state(Eye::Right);
    let unmapped = RenderView::new(UNMAPPED);
    assert_eq!(begin(&mut camera, unmapped), Err(EyeFault::ReadStateUnreadable));
    let lost = TestView::new(UNMAPPED.get(), &block_at(0.0));
    assert_eq!(begin(&mut camera, lost.view()), Err(EyeFault::ControllerUnreadable));
    assert_eq!(lost.pointer_at(VIEW_READ_STATE_CONTROLLER), UNMAPPED.get(), "the view as it was");
    let ((), text) = logged("eye-camera-fault", || {
        camera.on_view_begin(unmapped, view_writes(unmapped));
        camera.on_view_begin(unmapped, view_writes(unmapped));
    });
    let message = "eye camera read or write of game memory failed for this frame";
    let warnings: Vec<&str> = text.lines().filter(|line| line.contains(message)).collect();
    let [warning] = warnings.as_slice() else { panic!("one warning: {text}") };
    let fault = "fault=\"the view's ReadState controller pointer could not be read\"";
    assert!(warning.contains(" WARN  ") && warning.ends_with(fault), "{warning}");
}

#[test]
fn an_unreadable_last_seen_field_leaves_the_view_as_it_was() {
    let head = HeadController::new(WORLD_TRANSLATE);
    // Windows reserves address space in 64 KiB units, so the bytes past these 12 KiB up to the next
    // 64 KiB boundary belong to no allocation, and the last-seen field lies there.
    let pages = SharedPages::commit(VIEW_PAGES_LEN).expect("commit the test pages");
    let view = view_ending_before(at(pages.address(), VIEW_PAGES_LEN), &head);
    let mut camera = eye_state(Eye::Right);
    assert_eq!(begin(&mut camera, view), Err(EyeFault::LastSeenUnreadable));
    let read_state = try_read::<usize>(at(view.address(), VIEW_READ_STATE_CONTROLLER));
    assert_eq!(read_state, Some(head.address().get()), "the view as it was");
}

#[test]
fn a_failed_read_state_write_leaves_the_view_and_the_history_as_they_were() {
    let head = HeadController::new(WORLD_TRANSLATE);
    // The ReadState field and the last-seen field lie in the read-only page.
    let pages = ReadOnlyTail::new(3, &[head.address().get()]);
    let view_address = pages.read_only_start().get().checked_sub(VIEW_READ_STATE_CONTROLLER);
    let view = RenderView::new(GameAddr::new(view_address.expect("a test address")));
    let mut camera = eye_state(Eye::Left);
    assert_eq!(begin(&mut camera, view), Err(EyeFault::ReadStateWrite));
    let read_state = try_read::<usize>(pages.read_only_start());
    assert_eq!(read_state, Some(head.address().get()), "the view as it was");
    assert_eq!(camera.main.last, None, "a failed frame is no last proxied frame");
}

#[test]
fn a_failed_proxy_write_leaves_the_view_and_the_history_as_they_were() {
    let head = HeadController::new(WORLD_TRANSLATE);
    let view = TestView::new(head.address().get(), &block_at(0.0));
    let mut camera = eye_state(Eye::Right);
    camera.pages = ReadOnlyTail::new(0, &[]).into_pages();
    assert_eq!(begin(&mut camera, view.view()), Err(EyeFault::ProxyWrite));
    assert_eq!(view.pointer_at(VIEW_READ_STATE_CONTROLLER), head.address().get(), "the view");
    assert_eq!(view.pointer_at(VIEW_LAST_SEEN_CONTROLLER), 0, "the last-seen field as it was");
    assert_eq!(camera.main.last, None, "a failed frame is no last proxied frame");
}

#[test]
fn a_failed_last_seen_write_points_the_read_state_back_at_the_head() {
    let head = HeadController::new(WORLD_TRANSLATE);
    // The ReadState field is the last pointer of the read-write pages, and the last-seen field lies
    // in the read-only page.
    let mut camera = eye_state(Eye::Left);
    let first = at(camera.pages.address(), camera.main.ring[0]);
    // The read-only page holds the first proxy as the last-seen controller, as the engine's
    // begin-recording of the first frame would store it.
    let tail_offset = VIEW_LAST_SEEN_CONTROLLER.checked_sub(VIEW_READ_STATE_CONTROLLER + 8);
    let tail_index = tail_offset.and_then(|bytes| bytes.checked_div(size_of::<usize>()));
    let mut tail = vec![0; tail_index.expect("a word index")];
    tail.push(first.get());
    let pages = ReadOnlyTail::new(3, &tail);
    let view = view_ending_before(pages.read_only_start(), &head);
    assert_eq!(view.last_seen_controller(), Some(first), "the last-seen field in the tail");
    proxied(begin(&mut camera, view), History::Cut);
    let read_state = view_writes(view).offset(VIEW_READ_STATE_CONTROLLER).expect("the field");
    try_write(read_state, head.address().get()).expect("the frame sync");
    let read_state = read_state.start();
    camera.end_frame(Layout::MainOnly);
    assert_eq!(begin(&mut camera, view), Err(EyeFault::LastSeenWrite));
    assert_eq!(try_read::<usize>(read_state), Some(head.address().get()), "the view as it was");
}

#[test]
fn a_failed_restore_is_its_own_fault() {
    assert_eq!(last_seen_fault(Ok(())), EyeFault::LastSeenWrite);
    let fault = Err(WriteError::Faulted { address: 0x10 });
    assert_eq!(last_seen_fault(fault), EyeFault::RestoreWrite);
}

#[test]
#[cfg_attr(not(debug_assertions), ignore = "the counting allocator exists only in debug builds")]
fn the_frame_path_makes_no_heap_allocation() {
    let head = HeadController::new(WORLD_TRANSLATE);
    let view = TestView::new(head.address().get(), &block_at(0.0));
    let mut camera = eye_state(Eye::Right);
    camera.on_view_begin(view.view(), view.writes());
    view.finish_frame(head.address().get());
    let unmapped = RenderView::new(UNMAPPED);
    let guard = arm_alloc_guard().expect("arm the allocation guard");
    camera.end_frame(Layout::MainOnly);
    camera.on_view_begin(view.view(), view.writes());
    camera.end_frame(Layout::MainOnly);
    camera.on_view_begin(unmapped, view_writes(unmapped));
    assert_eq!(guard.count(), 0, "the eye camera's frame allocated");
    let read_state = GameAddr::new(view.pointer_at(VIEW_READ_STATE_CONTROLLER));
    let slot_9 = CameraController::new(read_state).world_translate_fn();
    assert_eq!(slot_9, Some(proxy::world_translate_fn()), "the guarded frame proxied the view");
    assert_eq!(view.pointer_at(VIEW_LAST_SEEN_CONTROLLER), read_state.get(), "a kept history");
}
