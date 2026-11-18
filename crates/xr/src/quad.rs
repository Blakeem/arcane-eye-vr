//! The screen quad: the one pose and the one size that both eyes' quad layers share.
//!
//! OpenXR's LOCAL space has its origin where the runtime sets it, at the head's position when the
//! application started or at a calibrated zero position such as SteamVR's seated position. Its Y
//! axis is up and its -Z axis is forward. A quad with the identity orientation is upright and faces +Z, so it
//! faces a viewer at the origin.

use openxr::{Extent2Df, Posef, Quaternionf, Vector3f};

/// The pose and the size of the flat screen in the LOCAL space, in meters.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScreenQuad {
    pub pose: Posef,
    pub size: Extent2Df,
}

/// The screen `distance_m` in front of the origin and `width_m` wide that shows a `frame` of
/// `(width, height)` pixels. The height keeps the frame's aspect ratio, since each half of a half
/// side by side pair holds the entire picture at half its width.
///
/// Returns `None` when a side of `frame` is 0 or above 65535 pixels, which no D3D11 texture
/// reaches.
pub fn screen(distance_m: f32, width_m: f32, frame: (u32, u32)) -> Option<ScreenQuad> {
    let (Some(frame_width), Some(frame_height)) = (pixels(frame.0), pixels(frame.1)) else {
        return None;
    };
    let height_m = width_m * frame_height / frame_width;
    let position = Vector3f { x: 0.0, y: 0.0, z: -distance_m };
    Some(ScreenQuad {
        pose: Posef { orientation: Quaternionf::IDENTITY, position },
        size: Extent2Df { width: width_m, height: height_m },
    })
}

/// A side length as a float, or `None` when it is 0 or above 65535.
fn pixels(side: u32) -> Option<f32> {
    let Ok(side) = u16::try_from(side) else {
        return None;
    };
    (side > 0).then(|| f32::from(side))
}

#[cfg(test)]
mod tests {
    use super::screen;
    use openxr::Quaternionf;

    #[test]
    fn the_screen_stands_upright_straight_ahead_at_its_distance() {
        let quad = screen(2.5, 3.0, (2560, 1600)).expect("a frame");
        let position = quad.pose.position;
        assert_eq!((position.x, position.y, position.z), (0.0, 0.0, -2.5));
        let upright = Quaternionf { x: 0.0, y: 0.0, z: 0.0, w: 1.0 };
        assert_eq!(quad.pose.orientation, upright);
    }

    #[test]
    fn the_height_keeps_the_entire_frame_s_aspect_ratio() {
        let wide = screen(2.5, 3.0, (2560, 1600)).expect("a frame").size;
        assert_eq!((wide.width, wide.height), (3.0, 1.875));
        let uhd = screen(1.0, 4.0, (3840, 2160)).expect("a frame").size;
        assert_eq!((uhd.width, uhd.height), (4.0, 2.25));
    }

    #[test]
    fn an_empty_or_oversized_frame_gives_no_screen() {
        assert_eq!(screen(2.5, 3.0, (0, 1600)), None);
        assert_eq!(screen(2.5, 3.0, (2560, 0)), None);
        assert_eq!(screen(2.5, 3.0, (70_000, 1600)), None);
    }
}
