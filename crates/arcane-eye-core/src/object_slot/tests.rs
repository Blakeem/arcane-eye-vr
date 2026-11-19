//! The object slot's write against stand-in object data stages in test memory, and its install.

use diag::health::{Feature, FeatureState, Health};
use game_bindings::object_stage::ObjectStage;
use game_bindings::offsets::{OBJECT_STAGE_SLOT, OBJECT_STAGE_VIEW};
use game_bindings::render_view::RenderView;
use game_bindings::state::arm_alloc_guard;
use game_mem::{GameAddr, WriteDest, try_read_bytes, try_write, try_write_slice};
use settings::dev::{DevFlags, MainViewEye, SecondView, StereoFlags};

use super::{Outcome, SlotFault, SlotState, assign, install};
use crate::config::Config;
use crate::eye_view::Layout;
use crate::install::GameCode;
use crate::logged;
use crate::raw::SharedPages;
use crate::raw::test_support::test_dest;

/// The views the stand-in stages name. No read goes there.
const MAIN: RenderView = RenderView::new(GameAddr::new(0x7FF6_0000_1000));
const EYE: RenderView = RenderView::new(GameAddr::new(0x7FF6_0000_2000));
const THIRD: RenderView = RenderView::new(GameAddr::new(0x7FF6_0000_3000));

const SIDE_BY_SIDE: Layout = Layout::SideBySide { main: MAIN, eye: EYE };

/// Windows reserves the lowest 64 KiB of every process, so no read or write there succeeds.
const UNMAPPED: GameAddr = GameAddr::new(0x10);

/// The message of a failed or skipped install.
const NOT_INSTALLED: &str = "object slot hook not installed";

/// A stand-in object data stage: [`ObjectStage::WRITE_LEN`] bytes of 0xEE that name `view` and hold
/// slot index 0, as the game's execute leaves both views' stages.
struct TestStage {
    _pages: SharedPages,
    address: GameAddr,
}

impl TestStage {
    fn new(view: RenderView) -> Self {
        let pages = SharedPages::commit(ObjectStage::WRITE_LEN).expect("commit the stage pages");
        let address = pages.address();
        let stage = test_dest(address, ObjectStage::WRITE_LEN);
        try_write_slice(stage, &[0xEE_u8; ObjectStage::WRITE_LEN]).expect("fill the stage");
        let view_field = stage.offset(OBJECT_STAGE_VIEW).expect("the view field");
        try_write(view_field, view.address().get()).expect("write the stage's view");
        try_write(stage.offset(OBJECT_STAGE_SLOT).expect("the slot field"), 0_i32).expect("slot");
        Self { _pages: pages, address }
    }

    fn stage(&self) -> ObjectStage {
        ObjectStage::new(self.address)
    }

    fn writes(&self) -> WriteDest<'static> {
        test_dest(self.address, ObjectStage::WRITE_LEN)
    }

    fn bytes(&self) -> Vec<u8> {
        try_read_bytes(self.address, ObjectStage::WRITE_LEN).expect("readable stage bytes")
    }
}

fn config(second_view: SecondView) -> Config {
    let stereo = StereoFlags { main_view_eye: MainViewEye::Center, second_view };
    Config::of_dev(DevFlags { stereo, ..DevFlags::default() })
}

/// A registry where each of `features` installed.
fn running(features: &[Feature]) -> Health {
    let health = Health::new();
    for feature in features {
        health.install_ok(*feature);
    }
    health
}

#[test]
fn side_by_side_the_eye_views_stage_takes_slot_1_and_keeps_every_other_byte() {
    let eye = TestStage::new(EYE);
    let mut expected = eye.bytes();
    expected[OBJECT_STAGE_SLOT..][..4].copy_from_slice(&1_i32.to_ne_bytes());
    assert_eq!(assign(SIDE_BY_SIDE, eye.stage(), eye.writes()), Ok(Outcome::Assigned));
    assert_eq!(eye.bytes(), expected);
    assert_eq!(eye.stage().slot(), Some(1));
}

#[test]
fn the_stage_of_the_main_view_or_of_a_third_view_keeps_its_slot() {
    for view in [MAIN, THIRD] {
        let stage = TestStage::new(view);
        let before = stage.bytes();
        assert_eq!(assign(SIDE_BY_SIDE, stage.stage(), stage.writes()), Ok(Outcome::Unchanged));
        assert_eq!(stage.bytes(), before, "{view:?}");
    }
}

#[test]
fn a_frame_without_the_side_by_side_layout_changes_no_stage() {
    let eye = TestStage::new(EYE);
    let before = eye.bytes();
    assert_eq!(assign(Layout::MainOnly, eye.stage(), eye.writes()), Ok(Outcome::Unchanged));
    assert_eq!(eye.bytes(), before);
}

#[test]
fn an_unreadable_stage_is_a_fault() {
    let stage = ObjectStage::new(UNMAPPED);
    let writes = test_dest(UNMAPPED, ObjectStage::WRITE_LEN);
    assert_eq!(assign(SIDE_BY_SIDE, stage, writes), Err(SlotFault::ViewUnreadable));
}

#[test]
fn the_frame_end_names_the_eye_view_of_the_next_frame() {
    let eye = TestStage::new(EYE);
    let mut state = SlotState::default();
    state.on_job_start(eye.stage(), eye.writes());
    assert_eq!(eye.stage().slot(), Some(0), "the first frame has no layout");
    state.end_frame(SIDE_BY_SIDE);
    state.on_job_start(eye.stage(), eye.writes());
    assert_eq!(eye.stage().slot(), Some(1));
}

#[test]
fn only_the_first_slot_is_logged_and_a_fault_logs_one_warning() {
    let eye = TestStage::new(EYE);
    let mut state = SlotState::default();
    state.end_frame(SIDE_BY_SIDE);
    let ((), text) = logged("object-slot-lines", || {
        state.on_job_start(eye.stage(), eye.writes());
        state.on_job_start(eye.stage(), eye.writes());
        let unmapped = ObjectStage::new(UNMAPPED);
        state.on_job_start(unmapped, test_dest(UNMAPPED, ObjectStage::WRITE_LEN));
        state.on_job_start(unmapped, test_dest(UNMAPPED, ObjectStage::WRITE_LEN));
    });
    let lines: Vec<&str> = text.lines().collect();
    let [first, fault] = lines.as_slice() else { panic!("two lines: {text}") };
    assert!(first.contains(" INFO  "), "{first}");
    assert!(first.contains("object slot gives the eye view its own per-object slot"), "{first}");
    assert!(first.ends_with(" slot=1"), "{first}");
    assert!(fault.contains(" WARN  "), "{fault}");
    let fault_end = " fault=\"the stage's view pointer could not be read\"";
    assert!(fault.ends_with(fault_end), "{fault}");
}

#[test]
#[cfg_attr(not(debug_assertions), ignore = "the counting allocator exists only in debug builds")]
fn the_frame_path_makes_no_heap_allocation() {
    let (main, eye) = (TestStage::new(MAIN), TestStage::new(EYE));
    let mut state = SlotState::default();
    state.end_frame(SIDE_BY_SIDE);
    state.on_job_start(eye.stage(), eye.writes());
    let guard = arm_alloc_guard().expect("arm the allocation guard");
    state.end_frame(SIDE_BY_SIDE);
    state.on_job_start(main.stage(), main.writes());
    state.on_job_start(eye.stage(), eye.writes());
    assert_eq!(guard.count(), 0, "the object slot's frame allocated");
    drop(guard);
    assert_eq!(eye.stage().slot(), Some(1));
}

#[test]
fn second_view_off_installs_no_hook_and_logs_that() {
    let health = running(&[Feature::PresentHook, Feature::EyeView]);
    let code = GameCode::copy();
    let off = config(SecondView::Off);
    let (installed, text) = logged("object-slot-off", || install(&health, &code, off));
    assert!(installed.is_none(), "a hook with second_view off");
    assert_eq!(health.state(Feature::ObjectSlot), FeatureState::Pending);
    let lines: Vec<&str> = text.lines().collect();
    let [line] = lines.as_slice() else { panic!("one line: {text}") };
    let end = format!(" msg=\"{NOT_INSTALLED}\" second_view=false");
    assert!(line.contains(" INFO  ") && line.ends_with(&end), "{line}");
}

#[test]
fn a_missing_signature_or_needed_feature_installs_nothing_and_logs_the_cause() {
    let cases: [(&str, &[Feature], &str); 3] = [
        (
            "object-slot-install",
            &[Feature::PresentHook, Feature::EyeView],
            "the object_stage_job_start signature yields no address: the pattern has no match",
        ),
        (
            "object-slot-no-present",
            &[Feature::EyeView],
            "the Present call-site hook, which ends each frame of the object slot, is not running",
        ),
        (
            "object-slot-no-eye-view",
            &[Feature::PresentHook],
            "the eye view feature, which places the views side by side, is not running",
        ),
    ];
    for (name, features, cause) in cases {
        let health = running(features);
        let code = GameCode::copy();
        let (installed, text) = logged(name, || install(&health, &code, config(SecondView::On)));
        assert!(installed.is_none(), "{name}: a hook in the test exe");
        assert_eq!(health.state(Feature::ObjectSlot), FeatureState::Unavailable, "{name}");
        let failures: Vec<&str> =
            text.lines().filter(|line| line.contains(NOT_INSTALLED)).collect();
        let [failure] = failures.as_slice() else { panic!("one failure line: {text}") };
        let expected_end = format!(" error=\"{cause}\"");
        assert!(failure.contains(" WARN  ") && failure.ends_with(&expected_end), "{failure}");
    }
}
