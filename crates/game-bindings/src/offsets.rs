//! Byte offsets of the game struct fields the mod reads, for game build 4.1.1.7631656.
//!
//! Each was measured with a debugger breakpoint and matches bg3-vr's layout.

/// The camera controller pointer of a render view's `ReadState`, which the engine builds the view's
/// `PerView` and `PerCamera` blocks from.
pub const VIEW_READ_STATE_CONTROLLER: usize = 0x21D8;

/// `PerCamera` block 0 of a render view.
pub const VIEW_PER_CAMERA: usize = 0x2660;

/// The world-to-camera view matrix in a `PerCamera` block, 16 `f32` in column-major order.
pub const PER_CAMERA_VIEW: usize = 0x00;

/// The projection matrix in a `PerCamera` block, 16 `f32` in column-major order.
pub const PER_CAMERA_PROJECTION: usize = 0x40;

/// The near plane distance in a `PerCamera` block, one `f32`.
pub const PER_CAMERA_NEAR: usize = 0xC0;

/// The far plane distance in a `PerCamera` block, one `f32`.
pub const PER_CAMERA_FAR: usize = 0xC4;

/// The camera's world position in a `PerCamera` block, three `f32`.
pub const PER_CAMERA_POSITION: usize = 0xD0;

/// The camera controller's vtable slot 9, `GetWorldTranslate`.
pub const CONTROLLER_WORLD_TRANSLATE_SLOT: usize = 9 * size_of::<usize>();
