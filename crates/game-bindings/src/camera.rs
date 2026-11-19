//! A render view's `PerCamera` block and its decode into a camera: position, forward axis and lens.
//! A view holds two pairs of a `PerCamera` block and the `PerView` block built from it.
//!
//! The engine's space is left-handed with Y up and +Z forward. The view matrix is world-to-camera
//! with the camera's right, up and forward axes as the rows of its rotation. The projection is a
//! reversed-Z perspective projection.

use crate::offsets::{VIEW_PER_CAMERA, VIEW_PER_CAMERA_1, VIEW_PER_VIEW, VIEW_PER_VIEW_1};

/// One of a render view's two pairs of a `PerCamera` block and its `PerView` block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockPair {
    First,
    Second,
}

impl BlockPair {
    /// Both pairs, the first first.
    pub const BOTH: [Self; 2] = [Self::First, Self::Second];

    /// The byte offset of the pair's `PerCamera` block in a render view.
    pub const fn per_camera(self) -> usize {
        match self {
            Self::First => VIEW_PER_CAMERA,
            Self::Second => VIEW_PER_CAMERA_1,
        }
    }

    /// The byte offset of the pair's `PerView` block in a render view.
    pub const fn per_view(self) -> usize {
        match self {
            Self::First => VIEW_PER_VIEW,
            Self::Second => VIEW_PER_VIEW_1,
        }
    }
}

/// The fields of a `PerCamera` block that the decode reads, as the engine built them for one frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PerCamera {
    /// The world-to-camera matrix, 16 `f32` in column-major order.
    pub view: [f32; 16],
    /// The projection matrix, 16 `f32` in column-major order.
    pub projection: [f32; 16],
    pub near: f32,
    pub far: f32,
    /// The camera's world position.
    pub position: [f32; 3],
}

/// The camera a render view renders with.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    /// The world position.
    pub position: [f32; 3],
    /// The forward axis in world space.
    pub forward: [f32; 3],
    /// The vertical field of view in radians.
    pub fov_y_rad: f32,
    /// The view's width divided by its height.
    pub aspect: f32,
    pub near: f32,
    pub far: f32,
}

impl PerCamera {
    /// The block's camera, or `None` when the projection is not a finite perspective projection:
    /// element 11 is not 1, element 15 is not 0, or element 0 or 5 is zero or not finite.
    pub fn camera(&self) -> Option<Camera> {
        let [x_scale, _, _, _, _, y_scale, _, _, _, _, _, w_from_z, _, _, _, w_from_w] =
            self.projection;
        let [_, _, forward_x, _, _, _, forward_y, _, _, _, forward_z, _, _, _, _, _] = self.view;
        let scales_valid =
            [x_scale, y_scale].iter().all(|scale| scale.is_finite() && *scale != 0.0);
        // A bit compare, since clippy's float_cmp rejects `==` against 1.0. Only +1.0 has these bits.
        let perspective = w_from_z.to_bits() == 1.0_f32.to_bits() && w_from_w == 0.0;
        if !scales_valid || !perspective {
            return None;
        }
        Some(Camera {
            position: self.position,
            forward: [forward_x, forward_y, forward_z],
            fov_y_rad: 2.0 * y_scale.recip().atan(),
            aspect: y_scale / x_scale,
            near: self.near,
            far: self.far,
        })
    }
}

#[cfg(test)]
mod tests;
