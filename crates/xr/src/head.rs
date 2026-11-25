//! The head of one frame: the pose of the VIEW reference space in the LOCAL space and the position
//! and the field of view of each eye view the runtime locates, the VIEW space's one location per
//! frame, and the pure decision of when a read gives them.
//!
//! The VIEW space's origin is the middle of the two eyes and its orientation is the head's, also on
//! a headset whose two eye views are turned outward.

use std::cell::Cell;

use openxr::{
    D3D11, Fovf, Posef, ReferenceSpaceType, Space, SpaceLocation, Time, Vector3f, View,
    ViewStateFlags,
};
use stereo_core::eye::PerEye;

use crate::error::{XrError, XrStep};
use crate::input;

/// The session's VIEW reference space and its last location, so the controllers' input read and
/// the head read of one frame see one head.
pub(crate) struct ViewSpace {
    space: Space,
    last: Cell<Option<(Time, SpaceLocation)>>,
}

impl ViewSpace {
    /// Creates the VIEW reference space of `session`.
    ///
    /// # Errors
    ///
    /// Returns the [`XrError`] of the failed create.
    pub(crate) fn create(session: &openxr::Session<D3D11>) -> Result<Self, XrError> {
        let created = session.create_reference_space(ReferenceSpaceType::VIEW, Posef::IDENTITY);
        let space = XrError::check(XrStep::CreateViewSpace, created)?;
        Ok(Self { space, last: Cell::new(None) })
    }

    /// The location in `base` at `time`. Every caller passes the session's LOCAL space as `base`,
    /// and a call for the time of the last call gives the last call's location.
    ///
    /// # Errors
    ///
    /// Returns the [`XrError`] of the failed space location.
    pub(crate) fn locate(&self, base: &Space, time: Time) -> Result<SpaceLocation, XrError> {
        once_per_time(&self.last, time, || {
            XrError::check(XrStep::LocateSpace, self.space.locate(base, time))
        })
    }
}

/// The location `last` holds for `time`, else the one `locate` gives, which `last` then holds.
fn once_per_time(
    last: &Cell<Option<(Time, SpaceLocation)>>,
    time: Time,
    locate: impl FnOnce() -> Result<SpaceLocation, XrError>,
) -> Result<SpaceLocation, XrError> {
    if let Some((at, location)) = last.get()
        && at == time
    {
        return Ok(location);
    }
    let location = locate()?;
    last.set(Some((time, location)));
    Ok(location)
}

/// The head and its eye views at a frame's display time, in the LOCAL space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Head {
    /// The pose of the VIEW reference space.
    pub pose: Posef,
    /// The position of each eye view.
    pub eyes: PerEye<Vector3f>,
    /// The field of view of each eye view, as angles in radians from the view's -Z axis.
    pub fovs: PerEye<Fovf>,
}

/// The head of one read: `view`, the location of the VIEW space, and `flags` and `views`, the
/// located views of the stereo view configuration in the left then right order. `None` unless the
/// VIEW space's position and orientation and the views' positions are valid and the runtime gave
/// two views.
pub(crate) fn located(view: SpaceLocation, flags: ViewStateFlags, views: &[View]) -> Option<Head> {
    let pose = input::tracked(view)?;
    if !flags.contains(ViewStateFlags::POSITION_VALID) {
        return None;
    }
    let [left, right] = views else {
        return None;
    };
    let eyes = PerEye { left: left.pose.position, right: right.pose.position };
    Some(Head { pose, eyes, fovs: PerEye { left: left.fov, right: right.fov } })
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::f32::consts::FRAC_1_SQRT_2;

    use openxr::{
        Fovf, Posef, Quaternionf, SpaceLocation, SpaceLocationFlags, Time, Vector3f, View,
        ViewStateFlags, sys,
    };
    use stereo_core::eye::PerEye;

    use super::{Head, located, once_per_time};
    use crate::error::{XrError, XrStep};

    const HEAD: Posef = Posef {
        orientation: Quaternionf { x: 0.0, y: FRAC_1_SQRT_2, z: 0.0, w: FRAC_1_SQRT_2 },
        position: Vector3f { x: 0.1, y: 1.6, z: -0.2 },
    };

    fn view_space(location_flags: SpaceLocationFlags) -> SpaceLocation {
        SpaceLocation { location_flags, pose: HEAD }
    }

    /// An eye view at `x` whose field of view reaches 0.9 radians up, down and outward and 0.8
    /// radians inward, so the two eyes' fields of view differ.
    fn eye_view(x: f32) -> View {
        let position = Vector3f { x, y: 1.6, z: -0.2 };
        let (outward, inward) = (0.9, 0.8);
        let (angle_left, angle_right) =
            if x < 0.1 { (-outward, inward) } else { (-inward, outward) };
        let fov = Fovf { angle_left, angle_right, angle_up: 0.9, angle_down: -0.9 };
        View { pose: Posef { orientation: Quaternionf::IDENTITY, position }, fov }
    }

    fn tracked() -> SpaceLocation {
        view_space(SpaceLocationFlags::POSITION_VALID | SpaceLocationFlags::ORIENTATION_VALID)
    }

    fn views_valid() -> ViewStateFlags {
        ViewStateFlags::POSITION_VALID | ViewStateFlags::ORIENTATION_VALID
    }

    #[test]
    fn a_tracked_head_and_two_placed_views_give_the_head_pose_and_each_eye_s_position_and_lens() {
        let views = [eye_view(0.068), eye_view(0.132)];
        let head = located(tracked(), views_valid(), &views).expect("a head");
        let eyes = PerEye {
            left: Vector3f { x: 0.068, y: 1.6, z: -0.2 },
            right: Vector3f { x: 0.132, y: 1.6, z: -0.2 },
        };
        let fovs = PerEye {
            left: Fovf { angle_left: -0.9, angle_right: 0.8, angle_up: 0.9, angle_down: -0.9 },
            right: Fovf { angle_left: -0.8, angle_right: 0.9, angle_up: 0.9, angle_down: -0.9 },
        };
        assert_eq!(head, Head { pose: HEAD, eyes, fovs });
    }

    #[test]
    fn an_untracked_head_or_unplaced_views_give_no_head() {
        let views = [eye_view(-0.032), eye_view(0.032)];
        let position_only = view_space(SpaceLocationFlags::POSITION_VALID);
        assert_eq!(located(position_only, views_valid(), &views), None, "no head orientation");
        let orientation_only = view_space(SpaceLocationFlags::ORIENTATION_VALID);
        assert_eq!(located(orientation_only, views_valid(), &views), None, "no head position");
        let turned_only = ViewStateFlags::ORIENTATION_VALID;
        assert_eq!(located(tracked(), turned_only, &views), None, "no view position");
    }

    #[test]
    fn a_view_count_other_than_two_gives_no_head() {
        let one = [eye_view(0.0)];
        assert_eq!(located(tracked(), views_valid(), &one), None);
        let three = [eye_view(-0.032), eye_view(0.0), eye_view(0.032)];
        assert_eq!(located(tracked(), views_valid(), &three), None);
        assert_eq!(located(tracked(), views_valid(), &[]), None);
    }

    #[test]
    fn the_view_space_is_located_once_per_time_and_a_failed_location_is_not_kept() {
        let (last, calls) = (Cell::new(None), Cell::new(0_u32));
        let (first, second) = (Time::from_nanos(100), Time::from_nanos(200));
        let failed = XrError { step: XrStep::LocateSpace, code: sys::Result::ERROR_TIME_INVALID };
        let both = SpaceLocationFlags::POSITION_VALID | SpaceLocationFlags::ORIENTATION_VALID;
        let none = SpaceLocationFlags::EMPTY;
        let locate = |time, result: Result<SpaceLocation, XrError>| {
            let located = once_per_time(&last, time, || {
                calls.set(calls.get().saturating_add(1));
                result
            });
            (located.map(|location| location.location_flags), calls.get())
        };
        assert_eq!(locate(first, Ok(tracked())), (Ok(both), 1), "the first location");
        assert_eq!(locate(first, Err(failed)), (Ok(both), 1), "the same time is located once");
        assert_eq!(locate(second, Err(failed)), (Err(failed), 2), "a new time");
        let untracked = view_space(none);
        assert_eq!(locate(second, Ok(untracked)), (Ok(none), 3), "a failure is located again");
    }
}
