//! The UI depth's decisions: the UI disparity and the eye scale that a nearest distance asks for, and
//! the values in use that follow them.
//!
//! Disparities are shares of a half's width. A scene point `z` meters away has the crossed disparity
//! `lens * eye_m / (2 * z)` between the two halves at the eye distance `eye_m`, where `lens` is the
//! main view's `1 / (aspect * tan(fov_y / 2))`. The depth offset moves the right half right, which
//! takes the offset off that disparity. In the game the disparity this formula predicts matched the
//! measured one within 0.2 percent of a half's width.

use std::time::Duration;

use game_bindings::camera::Camera;
use gfx::nearest::Nearest;
use settings::player::StereoSettings;
use stereo_core::eye::EyeScale;

/// The share the UI sits in front of the scene under it.
const MARGIN: f32 = 0.0025;

/// The largest UI disparity. Past it the eye distance shrinks.
const LIMIT: f32 = 0.04;

/// The UI disparity in use falls only while its target lies more than this share below it.
const HOLD: f32 = 0.0025;

/// The largest fall of the UI disparity in use in one second.
const FALL_PER_SECOND: f32 = 0.01;

/// The largest rise of the eye scale in use in one second.
const RISE_PER_SECOND: f32 = 0.5;

/// The divisor that turns a percentage into a share.
const PERCENT: f32 = 100.0;

/// The millimeters in one meter, the unit of the eye distance.
const MILLIMETERS_PER_METER: f32 = 1000.0;

/// The UI disparity and the eye scale of one moment, as targets or as the values in use.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Depth {
    /// The crossed disparity between the UI in the left half and in the right half, 0 or more.
    pub(crate) disparity: f32,
    pub(crate) eye_scale: EyeScale,
}

impl Depth {
    /// The UI on the screen surface and the eyes at the full distance.
    pub(crate) const FLAT: Self = Self { disparity: 0.0, eye_scale: EyeScale::FULL };
}

/// What turns a nearest distance into a disparity: the main view's lens, the full eye distance and
/// the depth offset.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct View {
    /// `1 / (aspect * tan(fov_y / 2))`.
    pub(crate) lens: f32,
    pub(crate) eye_m: f32,
    /// The depth offset as a share.
    pub(crate) offset: f32,
}

impl View {
    /// The view of the main view's `camera` and the player's `stereo` settings.
    pub(crate) fn of(camera: &Camera, stereo: StereoSettings) -> Self {
        Self {
            lens: (camera.aspect * (camera.fov_y_rad / 2.0).tan()).recip(),
            eye_m: stereo.ipd_mm.millimeters() / MILLIMETERS_PER_METER,
            offset: stereo.depth_offset_percent.percent() / PERCENT,
        }
    }
}

/// The targets that `nearest` asks for in `view`. A UI over no scene, a distance that is not above
/// 0 and a lens that gives no finite disparity above 0 all ask for [`Depth::FLAT`].
pub(crate) fn targets(nearest: Nearest, view: View) -> Depth {
    let Nearest::At(distance) = nearest else { return Depth::FLAT };
    if !distance.is_finite() || distance <= 0.0 {
        return Depth::FLAT;
    }
    let crossed = view.lens * view.eye_m / (2.0 * distance);
    if !crossed.is_finite() || crossed <= 0.0 {
        return Depth::FLAT;
    }
    let needed = crossed - view.offset + MARGIN;
    if needed.is_nan() || needed <= 0.0 {
        return Depth::FLAT;
    }
    if needed <= LIMIT {
        return Depth { disparity: needed, eye_scale: EyeScale::FULL };
    }
    let eye_scale = EyeScale::clamped((LIMIT + view.offset - MARGIN) / crossed);
    Depth { disparity: LIMIT, eye_scale }
}

/// The values in use after a frame that brought `target`, `elapsed` after the last step. A rise of
/// the UI disparity and a fall of the eye scale take effect at once. The eye cameras render at the
/// eye scale from the second frame after, so in the two frames between, a scene nearer than the
/// limit can sit in front of the UI. The other ways move at most at their rate, and the UI
/// disparity holds while its target lies within [`HOLD`] below it.
pub(crate) fn follow(in_use: Depth, target: Depth, elapsed: Duration) -> Depth {
    let seconds = elapsed.as_secs_f32();
    let gap = in_use.disparity - target.disparity;
    let disparity = if gap < 0.0 {
        target.disparity
    } else if gap > HOLD {
        (in_use.disparity - FALL_PER_SECOND * seconds).max(target.disparity)
    } else {
        in_use.disparity
    };
    let eye_scale = if target.eye_scale < in_use.eye_scale {
        target.eye_scale
    } else {
        let risen = in_use.eye_scale.share() + RISE_PER_SECOND * seconds;
        EyeScale::clamped(risen.min(target.eye_scale.share()))
    };
    Depth { disparity, eye_scale }
}

#[cfg(test)]
mod tests;
