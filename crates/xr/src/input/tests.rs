//! The decisions of the input read: the sync's focus, the tracked poses and the idle values.

use openxr::{
    ActionState, Posef, Quaternionf, SpaceLocation, SpaceLocationFlags, Time, Vector2f, Vector3f,
    sys,
};

use super::{ButtonState, Focus, HandInput, Input, amount, button, stick, synced, tracked};
use crate::error::{XrError, XrStep};

fn state<T: openxr::ActionInput>(current_state: T, is_active: bool) -> ActionState<T> {
    ActionState {
        current_state,
        changed_since_last_sync: false,
        last_change_time: Time::from_nanos(0),
        is_active,
    }
}

fn located(location_flags: SpaceLocationFlags) -> SpaceLocation {
    let pose = Posef {
        orientation: Quaternionf { x: 0.0, y: 1.0, z: 0.0, w: 0.0 },
        position: Vector3f { x: 0.2, y: -0.3, z: -0.4 },
    };
    SpaceLocation { location_flags, pose }
}

#[test]
fn a_sync_that_reports_no_focus_is_no_error_and_a_failed_sync_is_one() {
    assert_eq!(synced(sys::Result::SUCCESS), Ok(Focus::Focused));
    assert_eq!(synced(sys::Result::SESSION_NOT_FOCUSED), Ok(Focus::Unfocused));
    let lost = XrError { step: XrStep::SyncActions, code: sys::Result::ERROR_SESSION_LOST };
    assert_eq!(synced(sys::Result::ERROR_SESSION_LOST), Err(lost));
}

#[test]
fn a_pose_needs_both_its_position_and_its_orientation_valid() {
    let both = SpaceLocationFlags::POSITION_VALID | SpaceLocationFlags::ORIENTATION_VALID;
    let pose = tracked(located(both)).expect("a valid pose");
    assert_eq!((pose.position.x, pose.position.y, pose.position.z), (0.2, -0.3, -0.4));
    assert_eq!(tracked(located(SpaceLocationFlags::POSITION_VALID)), None);
    assert_eq!(tracked(located(SpaceLocationFlags::ORIENTATION_VALID)), None);
    assert_eq!(tracked(located(SpaceLocationFlags::EMPTY)), None);
    let tracked_only =
        SpaceLocationFlags::POSITION_TRACKED | SpaceLocationFlags::ORIENTATION_TRACKED;
    assert_eq!(tracked(located(tracked_only)), None, "tracked bits without valid bits");
}

#[test]
fn an_inactive_action_gives_its_idle_value() {
    assert_eq!(button(state(true, false)), ButtonState::Released);
    assert_eq!(amount(state(0.9, false)), 0.0);
    let scroll = stick(state(Vector2f { x: 0.5, y: -0.5 }, false));
    assert_eq!((scroll.x, scroll.y), (0.0, 0.0));
}

#[test]
fn an_active_action_gives_its_state() {
    assert_eq!(button(state(true, true)), ButtonState::Pressed);
    assert_eq!(button(state(false, true)), ButtonState::Released);
    assert_eq!(amount(state(0.75, true)), 0.75);
    let scroll = stick(state(Vector2f { x: 0.5, y: -0.25 }, true));
    assert_eq!((scroll.x, scroll.y), (0.5, -0.25));
}

#[test]
fn values_outside_their_range_are_clamped_and_a_value_that_is_not_a_number_is_0() {
    assert_eq!(amount(state(1.5, true)), 1.0);
    assert_eq!(amount(state(-0.5, true)), 0.0);
    assert_eq!(amount(state(f32::NAN, true)), 0.0);
    let scroll = stick(state(Vector2f { x: -3.0, y: f32::NAN }, true));
    assert_eq!((scroll.x, scroll.y), (-1.0, 0.0));
}

#[test]
fn the_idle_input_has_no_pose_nothing_pressed_and_0() {
    let idle = Input::default();
    for hand in [idle.left, idle.right] {
        assert_eq!(
            hand,
            HandInput {
                aim: None,
                grip: None,
                menu: ButtonState::Released,
                select: 0.0,
                secondary: ButtonState::Released,
                scroll: Vector2f { x: 0.0, y: 0.0 },
            }
        );
    }
    assert_eq!(idle.head, None);
}
