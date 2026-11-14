//! Byte offsets and sizes of the game struct fields the mod reads and writes, for game build
//! 4.1.1.7631656.
//!
//! Each matches bg3-vr's layout. The view and `PerCamera` offsets were measured with a debugger
//! breakpoint. The camera-cut field was checked by a static disassembly of the installed exe.

/// The camera controller pointer of a render view's `ReadState`, which the engine builds the view's
/// `PerView` and `PerCamera` blocks from.
pub const VIEW_READ_STATE_CONTROLLER: usize = 0x21D8;

/// The controller pointer a render view saw last. Begin-recording sets the view's camera-cut flag when
/// it differs from the `ReadState` controller pointer, then stores that pointer here.
pub const VIEW_LAST_SEEN_CONTROLLER: usize = 0x2B00;

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

/// The size of the gameplay camera controller object.
pub const CONTROLLER_SIZE: usize = 0x330;

/// The world-to-camera view matrix in a camera controller, 16 `f32` in column-major order.
pub const CONTROLLER_VIEW: usize = 0x90;

/// The inverse of the view matrix in a camera controller, 16 `f32` in column-major order.
pub const CONTROLLER_INVERSE_VIEW: usize = 0xD0;

/// The number of camera controller vtable slots a copy of the vtable holds, as bg3-vr's proxy copies.
pub const CONTROLLER_VTABLE_SLOTS: usize = 96;

/// The camera controller's vtable slot 9, `GetWorldTranslate`.
pub const CONTROLLER_WORLD_TRANSLATE_SLOT: usize = 9 * size_of::<usize>();

const _: () = assert!(CONTROLLER_INVERSE_VIEW + 16 * size_of::<f32>() <= CONTROLLER_SIZE);
const _: () =
    assert!(CONTROLLER_WORLD_TRANSLATE_SLOT < CONTROLLER_VTABLE_SLOTS * size_of::<usize>());
