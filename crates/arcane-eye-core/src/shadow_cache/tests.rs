//! The shadow cache's copy against stand-in shadow stages in test memory, and its install.

mod install;

use game_bindings::offsets::{
    SHADOW_CACHE_ON, SHADOW_CACHE_PARITY, SHADOW_CACHE_RECORDS, SHADOW_CACHE_RECORDS_SIZE,
    SHADOW_STAGE_VIEW,
};
use game_bindings::render_view::RenderView;
use game_bindings::shadow_stage::ShadowStage;
use game_bindings::state::arm_alloc_guard;
use game_mem::{GameAddr, WriteDest, try_read_bytes, try_write, try_write_slice};

use super::{Outcome, ShadowFault, ShadowState};
use crate::eye_view::Layout;
use crate::logged;
use crate::raw::SharedPages;
use crate::raw::test_support::{PAGE_SIZE, ReadOnlyTail, test_dest};

/// The views the stand-in stages name. No read goes there.
const MAIN: RenderView = RenderView::new(GameAddr::new(0x7FF6_0000_1000));
const EYE: RenderView = RenderView::new(GameAddr::new(0x7FF6_0000_2000));
const THIRD: RenderView = RenderView::new(GameAddr::new(0x7FF6_0000_3000));

const SIDE_BY_SIDE: Layout = Layout::SideBySide { main: MAIN, eye: EYE };

/// Windows reserves the lowest 64 KiB of every process, so no read or write there succeeds.
const UNMAPPED: GameAddr = GameAddr::new(0x10);

/// The parity byte and the on byte of the main view's stand-in stage, which differ from each other
/// and from the eye view's.
const MAIN_PARITY: u8 = 1;
const MAIN_ON: u8 = 0xA5;

/// The bytes of the read-only page that the stage of the read-only test holds before its on byte.
const ON_IN_TAIL: usize = 3;

/// The first word of that read-only page, which holds 0x5A at the on byte.
const TAIL_WORD: usize = 0x5A << (8 * ON_IN_TAIL);

/// A stand-in shadow stage: [`ShadowStage::WRITE_LEN`] bytes that all differ from a stage of
/// another `seed`, with its view pointer at the stage's view field.
struct TestStage {
    _pages: Option<SharedPages>,
    address: GameAddr,
}

impl TestStage {
    fn new(view: RenderView, seed: usize) -> Self {
        let pages = SharedPages::commit(ShadowStage::WRITE_LEN).expect("commit the stage pages");
        let address = pages.address();
        fill(address, ShadowStage::WRITE_LEN, view, seed);
        Self { _pages: Some(pages), address }
    }

    /// A stage at `address` in memory its caller holds, whose first `writable` bytes it fills as
    /// [`TestStage::new`] fills a stage.
    fn at(address: GameAddr, writable: usize, view: RenderView, seed: usize) -> Self {
        fill(address, writable, view, seed);
        Self { _pages: None, address }
    }

    fn stage(&self) -> ShadowStage {
        ShadowStage::new(self.address)
    }

    fn writes(&self) -> WriteDest<'static> {
        test_dest(self.address, ShadowStage::WRITE_LEN)
    }

    fn bytes(&self) -> Vec<u8> {
        try_read_bytes(self.address, ShadowStage::WRITE_LEN).expect("readable stage bytes")
    }
}

/// Fills the `len` bytes of a stage at `start` with a pattern that starts at `seed`, and names
/// `view`.
fn fill(start: GameAddr, len: usize, view: RenderView, seed: usize) {
    let stage = test_dest(start, len);
    let pattern: Vec<u8> = (0..=u8::MAX).cycle().skip(seed).take(len).collect();
    try_write_slice(stage, &pattern).expect("fill the stage");
    let view_field = stage.offset(SHADOW_STAGE_VIEW).expect("the view field");
    try_write(view_field, view.address().get()).expect("write the stage's view");
}

/// The main view's stage, with its own parity byte and on byte.
fn main_stage() -> TestStage {
    let stage = TestStage::new(MAIN, 0);
    let writes = stage.writes();
    try_write(writes.offset(SHADOW_CACHE_PARITY).expect("the parity"), MAIN_PARITY)
        .expect("parity");
    try_write(writes.offset(SHADOW_CACHE_ON).expect("the on byte"), MAIN_ON).expect("on byte");
    stage
}

/// The bytes of `eye` with the parity byte, the records and the on byte of `main`.
fn with_cache_of(eye: &[u8], main: &[u8]) -> Vec<u8> {
    let mut expected = eye.to_vec();
    expected[SHADOW_CACHE_PARITY] = main[SHADOW_CACHE_PARITY];
    let records = SHADOW_CACHE_RECORDS..SHADOW_CACHE_RECORDS + SHADOW_CACHE_RECORDS_SIZE;
    expected[records.clone()].copy_from_slice(&main[records]);
    expected[SHADOW_CACHE_ON] = main[SHADOW_CACHE_ON];
    expected
}

/// A shadow cache state whose current frame places the views side by side.
fn side_by_side_state() -> ShadowState {
    let mut state = ShadowState::default();
    state.end_frame(SIDE_BY_SIDE);
    state
}

/// What the handler does with `stand_in` at the exit of its execute.
fn exit(state: &mut ShadowState, stand_in: &TestStage) -> Result<Outcome, ShadowFault> {
    state.share(stand_in.stage(), stand_in.writes())
}

#[test]
fn side_by_side_the_eye_views_stage_takes_the_main_views_cache_state_and_keeps_every_other_byte() {
    let (main, eye) = (main_stage(), TestStage::new(EYE, 7));
    let (main_before, eye_before) = (main.bytes(), eye.bytes());
    assert_ne!(eye_before[SHADOW_CACHE_PARITY], MAIN_PARITY, "the stages' parities differ");
    assert_ne!(eye_before[SHADOW_CACHE_ON], MAIN_ON, "the stages' on bytes differ");
    let mut state = side_by_side_state();
    assert_eq!(exit(&mut state, &main), Ok(Outcome::Unchanged));
    let copied = Outcome::Copied { parity: MAIN_PARITY, on: MAIN_ON };
    assert_eq!(exit(&mut state, &eye), Ok(copied));
    assert_eq!(eye.bytes(), with_cache_of(&eye_before, &main_before), "the eye view's stage");
    assert_eq!(main.bytes(), main_before, "the main view's stage as it was");
}

#[test]
fn the_eye_views_stage_before_the_main_views_in_a_frame_keeps_its_state() {
    let (main, eye) = (main_stage(), TestStage::new(EYE, 7));
    let eye_before = eye.bytes();
    let mut state = side_by_side_state();
    assert_eq!(exit(&mut state, &eye), Ok(Outcome::Unchanged));
    assert_eq!(exit(&mut state, &main), Ok(Outcome::Unchanged));
    assert_eq!(eye.bytes(), eye_before);
}

#[test]
fn a_frame_without_the_side_by_side_layout_copies_nothing() {
    let (main, eye) = (main_stage(), TestStage::new(EYE, 7));
    let eye_before = eye.bytes();
    let mut state = ShadowState::default();
    assert_eq!(exit(&mut state, &main), Ok(Outcome::Unchanged));
    assert_eq!(exit(&mut state, &eye), Ok(Outcome::Unchanged), "the first frame");
    state.end_frame(SIDE_BY_SIDE);
    assert_eq!(exit(&mut state, &main), Ok(Outcome::Unchanged));
    state.end_frame(Layout::MainOnly);
    assert_eq!(exit(&mut state, &main), Ok(Outcome::Unchanged));
    assert_eq!(exit(&mut state, &eye), Ok(Outcome::Unchanged), "a frame after a side by side one");
    assert_eq!(eye.bytes(), eye_before);
}

#[test]
fn a_third_views_stage_keeps_its_state_and_is_never_the_source() {
    let (main, eye) = (main_stage(), TestStage::new(EYE, 7));
    let third = TestStage::new(THIRD, 13);
    let (main_before, eye_before, third_before) = (main.bytes(), eye.bytes(), third.bytes());
    let mut state = side_by_side_state();
    assert_eq!(exit(&mut state, &third), Ok(Outcome::Unchanged), "before the main view");
    assert_eq!(exit(&mut state, &main), Ok(Outcome::Unchanged));
    assert_eq!(exit(&mut state, &third), Ok(Outcome::Unchanged), "after the main view");
    assert!(matches!(exit(&mut state, &eye), Ok(Outcome::Copied { .. })));
    assert_eq!(third.bytes(), third_before, "the third view's stage");
    assert_eq!(eye.bytes(), with_cache_of(&eye_before, &main_before), "the main view's state");
}

#[test]
fn the_frame_end_forgets_the_main_views_stage() {
    let (main, eye) = (main_stage(), TestStage::new(EYE, 7));
    let (main_before, eye_before) = (main.bytes(), eye.bytes());
    let mut state = side_by_side_state();
    assert_eq!(exit(&mut state, &main), Ok(Outcome::Unchanged));
    state.end_frame(SIDE_BY_SIDE);
    assert_eq!(exit(&mut state, &eye), Ok(Outcome::Unchanged), "no main view stage this frame");
    assert_eq!(eye.bytes(), eye_before);
    state.end_frame(SIDE_BY_SIDE);
    assert_eq!(exit(&mut state, &main), Ok(Outcome::Unchanged));
    assert!(matches!(exit(&mut state, &eye), Ok(Outcome::Copied { .. })), "the next frame");
    assert_eq!(eye.bytes(), with_cache_of(&eye_before, &main_before));
}

#[test]
fn only_the_first_copy_is_logged() {
    let (main, eye) = (main_stage(), TestStage::new(EYE, 7));
    let mut state = side_by_side_state();
    let ((), text) = logged("shadow-cache-first-copy", || {
        for _ in 0..2 {
            state.on_stage_exit(main.stage(), main.writes());
            state.on_stage_exit(eye.stage(), eye.writes());
            state.end_frame(SIDE_BY_SIDE);
        }
    });
    let lines: Vec<&str> = text.lines().collect();
    let [line] = lines.as_slice() else { panic!("one line: {text}") };
    let message = "shadow cache gives the eye view the main view's distant shadow cache";
    let end = format!(" msg=\"{message}\" stage={:#x} parity=1 on=165", eye.address.get());
    assert!(line.contains(" INFO  ") && line.ends_with(&end), "{line}");
}

#[test]
fn an_unreadable_stage_is_a_fault_with_one_warning_per_period() {
    let mut state = side_by_side_state();
    let unmapped = ShadowStage::new(UNMAPPED);
    let writes = test_dest(UNMAPPED, ShadowStage::WRITE_LEN);
    assert_eq!(state.share(unmapped, writes), Err(ShadowFault::ViewUnreadable));
    let ((), text) = logged("shadow-cache-fault", || {
        state.on_stage_exit(unmapped, writes);
        state.on_stage_exit(unmapped, writes);
    });
    let message = "shadow cache read or write of game memory failed for this frame";
    let warnings: Vec<&str> = text.lines().filter(|line| line.contains(message)).collect();
    let [warning] = warnings.as_slice() else { panic!("one warning: {text}") };
    let fault = "fault=\"the stage's view pointer could not be read\"";
    assert!(warning.contains(" WARN  ") && warning.ends_with(fault), "{warning}");
}

#[test]
fn an_unreadable_cache_state_of_either_stage_leaves_the_eye_views_stage_as_it_was() {
    // Windows reserves address space in 64 KiB units, so the bytes past one committed page belong to
    // no allocation, and the cache state of a stage at the page's start lies there.
    let short_pages = SharedPages::commit(PAGE_SIZE).expect("commit the short stage page");
    let main_dest = test_dest(short_pages.address(), PAGE_SIZE);
    try_write(main_dest.offset(SHADOW_STAGE_VIEW).expect("the view field"), MAIN.address().get())
        .expect("write the short stage's view");
    let short_main = ShadowStage::new(short_pages.address());
    let eye = TestStage::new(EYE, 7);
    let eye_before = eye.bytes();
    let mut state = side_by_side_state();
    assert_eq!(state.share(short_main, main_dest), Ok(Outcome::Unchanged));
    assert_eq!(exit(&mut state, &eye), Err(ShadowFault::MainUnreadable));
    assert_eq!(eye.bytes(), eye_before);

    let main = main_stage();
    try_write(main_dest.offset(SHADOW_STAGE_VIEW).expect("the view field"), EYE.address().get())
        .expect("name the eye view");
    state.end_frame(SIDE_BY_SIDE);
    assert_eq!(exit(&mut state, &main), Ok(Outcome::Unchanged));
    assert_eq!(state.share(short_main, main_dest), Err(ShadowFault::EyeUnreadable));
}

#[test]
fn a_read_only_on_byte_leaves_the_eye_views_stage_as_it_was() {
    // The read-only page starts ON_IN_TAIL bytes before the on byte, past the records and the
    // parity byte.
    let pages = ReadOnlyTail::new(3, &[TAIL_WORD]);
    let writable = SHADOW_CACHE_ON - ON_IN_TAIL;
    let start = pages.read_only_start().get().checked_sub(writable);
    let eye = TestStage::at(GameAddr::new(start.expect("a test address")), writable, EYE, 7);
    let main = main_stage();
    let eye_before = eye.bytes();
    assert_eq!(eye_before[SHADOW_CACHE_ON], 0x5A, "the on byte in the read-only page");
    let mut state = side_by_side_state();
    assert_eq!(exit(&mut state, &main), Ok(Outcome::Unchanged));
    assert_eq!(exit(&mut state, &eye), Err(ShadowFault::EyeWrite));
    assert_eq!(eye.bytes(), eye_before, "the records and the parity as the game set them");
}

#[test]
#[cfg_attr(not(debug_assertions), ignore = "the counting allocator exists only in debug builds")]
fn the_frame_path_makes_no_heap_allocation() {
    let (main, eye) = (main_stage(), TestStage::new(EYE, 7));
    let main_before = main.bytes();
    let mut state = side_by_side_state();
    state.on_stage_exit(main.stage(), main.writes());
    state.on_stage_exit(eye.stage(), eye.writes());
    let eye_parity = eye.writes().offset(SHADOW_CACHE_PARITY).expect("the parity");
    try_write(eye_parity, 0_u8).expect("the eye view's own parity again");
    let unmapped = ShadowStage::new(UNMAPPED);
    let guard = arm_alloc_guard().expect("arm the allocation guard");
    state.end_frame(SIDE_BY_SIDE);
    state.on_stage_exit(main.stage(), main.writes());
    state.on_stage_exit(eye.stage(), eye.writes());
    state.on_stage_exit(unmapped, test_dest(UNMAPPED, ShadowStage::WRITE_LEN));
    state.end_frame(Layout::MainOnly);
    assert_eq!(guard.count(), 0, "the shadow cache's frame allocated");
    drop(guard);
    let parity = eye.bytes()[SHADOW_CACHE_PARITY];
    assert_eq!(parity, main_before[SHADOW_CACHE_PARITY], "the guarded frame copied the state");
}
