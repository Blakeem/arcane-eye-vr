//! The stereo pair: the main view's proxy at the left eye and the eye view's at the right eye, each
//! from its own ring with its own camera-cut record.

use game_bindings::offsets::{VIEW_LAST_SEEN_CONTROLLER, VIEW_READ_STATE_CONTROLLER};
use game_bindings::render_view::CameraController;
use game_bindings::state::arm_alloc_guard;
use game_mem::GameAddr;
use stereo_core::eye::Eye;

use super::super::proxy::{self, RING_SIZE};
use super::super::test_support::{HALF_IPD_M, HeadController, target_state};
use super::super::{EyeFault, EyeState, History, Outcome, Target};
use super::{assert_near, begin, eye_position, floats};
use crate::eye_view::Layout;
use crate::raw::test_support::call_world_translate;
use crate::view_camera::test_support::{TestView, WORLD_TRANSLATE, block_at};

/// A main view, an eye view and a third view, each on `head`'s controller.
struct Views {
    main: TestView,
    eye: TestView,
    third: TestView,
}

impl Views {
    fn on(head: &HeadController) -> Self {
        let controller = head.address().get();
        Self {
            main: TestView::new(controller, &block_at(0.0)),
            eye: TestView::new(controller, &block_at(0.0)),
            third: TestView::new(controller, &block_at(0.0)),
        }
    }

    fn side_by_side(&self) -> Layout {
        Layout::SideBySide { main: self.main.view(), eye: self.eye.view() }
    }
}

/// A stereo pair state whose current frame places `views` side by side.
fn pair_state(views: &Views) -> EyeState {
    let mut camera = target_state(Target::StereoPair);
    camera.end_frame(views.side_by_side());
    camera
}

/// The proxy that `outcome` names, after checking that it is a proxied outcome at `eye` with
/// `history`.
fn proxied_at(outcome: Result<Outcome, EyeFault>, eye: Eye, history: History) -> GameAddr {
    match outcome {
        Ok(Outcome::Proxied { eye: got_eye, proxy, history: got, .. })
            if got_eye == eye && got == history =>
        {
            proxy
        }
        other => panic!("a proxied view at {eye:?} with {history:?}: {other:?}"),
    }
}

/// The ring that holds `proxy`: 0 for the first `RING_SIZE` bytes of `camera`'s pages, 1 for the
/// next.
fn ring_of(camera: &EyeState, proxy: GameAddr) -> usize {
    let offset =
        proxy.get().checked_sub(camera.pages.address().get()).expect("a proxy in the pages");
    offset.checked_div(RING_SIZE).expect("a ring index")
}

/// Ends the frame as the engine and the eye camera end it, with each view's `ReadState` back at
/// the controller its pair names.
fn finish_frame(camera: &mut EyeState, views: &Views, main: &HeadController, eye: &HeadController) {
    views.main.finish_frame(main.address().get());
    views.eye.finish_frame(eye.address().get());
    camera.end_frame(views.side_by_side());
}

#[test]
fn side_by_side_the_main_view_renders_the_left_eye_and_the_eye_view_the_right_eye() {
    let head = HeadController::new(WORLD_TRANSLATE);
    let views = Views::on(&head);
    let mut camera = pair_state(&views);
    let left = proxied_at(begin(&mut camera, views.main.view()), Eye::Left, History::Cut);
    let right = proxied_at(begin(&mut camera, views.eye.view()), Eye::Right, History::Cut);
    assert_eq!(begin(&mut camera, views.third.view()), Ok(Outcome::Unchanged));
    assert_eq!(views.main.pointer_at(VIEW_READ_STATE_CONTROLLER), left.get());
    assert_eq!(views.eye.pointer_at(VIEW_READ_STATE_CONTROLLER), right.get());
    assert_eq!(views.third.pointer_at(VIEW_READ_STATE_CONTROLLER), head.address().get());
    for (proxy, offset_m, eye) in [(left, -HALF_IPD_M, Eye::Left), (right, HALF_IPD_M, Eye::Right)]
    {
        let slot_9 = CameraController::new(proxy).world_translate_fn();
        assert_eq!(slot_9, Some(proxy::world_translate_fn()), "{eye:?} slot 9");
        let translate = call_world_translate(proxy::world_translate_fn(), proxy);
        assert_near(&floats::<3>(translate), &eye_position(offset_m), "the eye position");
        assert_eq!(proxy::eye(proxy), Some(eye), "the proxy's eye");
    }
    assert_eq!([ring_of(&camera, left), ring_of(&camera, right)], [0, 1], "a ring for each view");
}

#[test]
fn the_eye_view_keeps_its_role_when_it_begins_before_the_main_view() {
    let head = HeadController::new(WORLD_TRANSLATE);
    let views = Views::on(&head);
    let mut camera = pair_state(&views);
    let right = proxied_at(begin(&mut camera, views.eye.view()), Eye::Right, History::Cut);
    let left = proxied_at(begin(&mut camera, views.main.view()), Eye::Left, History::Cut);
    assert_eq!([ring_of(&camera, left), ring_of(&camera, right)], [0, 1]);
    assert_eq!(begin(&mut camera, views.third.view()), Ok(Outcome::Unchanged));
}

#[test]
fn each_view_keeps_its_history_by_its_own_last_proxied_frame() {
    let head = HeadController::new(WORLD_TRANSLATE);
    let other_head = HeadController::new(WORLD_TRANSLATE);
    let views = Views::on(&head);
    let mut camera = pair_state(&views);
    proxied_at(begin(&mut camera, views.main.view()), Eye::Left, History::Cut);
    proxied_at(begin(&mut camera, views.eye.view()), Eye::Right, History::Cut);
    finish_frame(&mut camera, &views, &head, &head);
    let left = proxied_at(begin(&mut camera, views.main.view()), Eye::Left, History::Kept);
    let right = proxied_at(begin(&mut camera, views.eye.view()), Eye::Right, History::Kept);
    assert_eq!(views.main.pointer_at(VIEW_LAST_SEEN_CONTROLLER), left.get());
    assert_eq!(views.eye.pointer_at(VIEW_LAST_SEEN_CONTROLLER), right.get());
    finish_frame(&mut camera, &views, &head, &other_head);
    let kept = proxied_at(begin(&mut camera, views.main.view()), Eye::Left, History::Kept);
    proxied_at(begin(&mut camera, views.eye.view()), Eye::Right, History::Cut);
    assert_eq!(views.main.pointer_at(VIEW_LAST_SEEN_CONTROLLER), kept.get(), "the main view");
    let eye_last_seen = views.eye.pointer_at(VIEW_LAST_SEEN_CONTROLLER);
    assert_eq!(eye_last_seen, right.get(), "a changed controller leaves the field as it was");
}

#[test]
fn a_frame_that_is_not_side_by_side_proxies_no_view() {
    let head = HeadController::new(WORLD_TRANSLATE);
    let views = Views::on(&head);
    let mut camera = target_state(Target::StereoPair);
    camera.end_frame(Layout::MainOnly);
    for view in [&views.main, &views.eye, &views.third] {
        assert_eq!(begin(&mut camera, view.view()), Ok(Outcome::Unchanged));
        assert_eq!(view.pointer_at(VIEW_READ_STATE_CONTROLLER), head.address().get());
    }
    camera.end_frame(views.side_by_side());
    proxied_at(begin(&mut camera, views.main.view()), Eye::Left, History::Cut);
}

#[test]
fn a_main_only_frame_end_after_a_side_by_side_frame_proxies_no_view() {
    let head = HeadController::new(WORLD_TRANSLATE);
    let views = Views::on(&head);
    let mut camera = target_state(Target::StereoPair);
    camera.end_frame(views.side_by_side());
    proxied_at(begin(&mut camera, views.main.view()), Eye::Left, History::Cut);

    let next = Views::on(&head);

    camera.end_frame(Layout::MainOnly);

    for view in [&next.main, &next.eye] {
        assert_eq!(begin(&mut camera, view.view()), Ok(Outcome::Unchanged));
        assert_eq!(view.pointer_at(VIEW_READ_STATE_CONTROLLER), head.address().get());
    }
    camera.end_frame(next.side_by_side());
    proxied_at(begin(&mut camera, next.eye.view()), Eye::Right, History::Cut);
}

#[test]
fn an_eye_view_on_another_controller_is_left_as_it_is() {
    let head = HeadController::new(WORLD_TRANSLATE);
    let views = Views::on(&head);
    let null = TestView::new(0, &block_at(0.0));
    let mut camera = target_state(Target::StereoPair);
    camera.end_frame(Layout::SideBySide { main: views.main.view(), eye: null.view() });
    assert_eq!(begin(&mut camera, null.view()), Ok(Outcome::Unchanged));
    assert_eq!(null.pointer_at(VIEW_READ_STATE_CONTROLLER), 0);
    proxied_at(begin(&mut camera, views.main.view()), Eye::Left, History::Cut);
}

#[test]
fn a_main_view_target_proxies_only_the_first_head_view_whatever_the_layout() {
    let head = HeadController::new(WORLD_TRANSLATE);
    let views = Views::on(&head);
    let mut camera = target_state(Target::MainView(Eye::Right));
    camera.end_frame(views.side_by_side());
    let proxy = proxied_at(begin(&mut camera, views.main.view()), Eye::Right, History::Cut);
    assert_eq!(ring_of(&camera, proxy), 0, "the main view's ring");
    assert_eq!(begin(&mut camera, views.eye.view()), Ok(Outcome::Unchanged));
    assert_eq!(views.eye.pointer_at(VIEW_READ_STATE_CONTROLLER), head.address().get());
}

#[test]
#[cfg_attr(not(debug_assertions), ignore = "the counting allocator exists only in debug builds")]
fn the_frame_path_of_two_views_makes_no_heap_allocation() {
    let head = HeadController::new(WORLD_TRANSLATE);
    let views = Views::on(&head);
    let mut camera = pair_state(&views);
    camera.on_view_begin(views.main.view(), views.main.writes());
    camera.on_view_begin(views.eye.view(), views.eye.writes());
    finish_frame(&mut camera, &views, &head, &head);
    let guard = arm_alloc_guard().expect("arm the allocation guard");
    camera.on_view_begin(views.main.view(), views.main.writes());
    camera.on_view_begin(views.eye.view(), views.eye.writes());
    camera.on_view_begin(views.third.view(), views.third.writes());
    camera.end_frame(views.side_by_side());
    assert_eq!(guard.count(), 0, "the eye camera's frame of two views allocated");
    for (view, eye) in [(&views.main, Eye::Left), (&views.eye, Eye::Right)] {
        let read_state = GameAddr::new(view.pointer_at(VIEW_READ_STATE_CONTROLLER));
        assert_eq!(proxy::eye(read_state), Some(eye), "the guarded frame proxied the view");
        assert_eq!(view.pointer_at(VIEW_LAST_SEEN_CONTROLLER), read_state.get(), "a kept history");
    }
}
