//! Typed, guarded reads of an engine render view and of the camera controller its `ReadState` names.
//!
//! Each read goes through `game-mem`'s guarded copy, so a wrong address yields `None` and never a
//! fault. A read checks no type, so a right address is the caller's part.

use game_mem::{GameAddr, try_read, try_read_array};

use crate::camera::PerCamera;
use crate::offsets::{
    CONTROLLER_WORLD_TRANSLATE_SLOT, PER_CAMERA_FAR, PER_CAMERA_NEAR, PER_CAMERA_POSITION,
    PER_CAMERA_PROJECTION, PER_CAMERA_VIEW, VIEW_PER_CAMERA, VIEW_READ_STATE_CONTROLLER,
};

/// A render view, by its address in the game process.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RenderView(GameAddr);

/// A camera controller, by its address in the game process.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CameraController(GameAddr);

impl RenderView {
    pub const fn new(address: GameAddr) -> Self {
        Self(address)
    }

    /// The camera controller the view's `ReadState` points to, or `None` when that pointer is null or
    /// cannot be read.
    pub fn read_state_controller(self) -> Option<CameraController> {
        let pointer = try_read::<usize>(self.0.checked_add(VIEW_READ_STATE_CONTROLLER)?)?;
        (pointer != 0).then_some(CameraController(GameAddr::new(pointer)))
    }

    /// The view's `PerCamera` block 0, or `None` when any field of it cannot be read.
    pub fn per_camera(self) -> Option<PerCamera> {
        let block = self.0.checked_add(VIEW_PER_CAMERA)?;
        Some(PerCamera {
            view: try_read_array(block.checked_add(PER_CAMERA_VIEW)?)?,
            projection: try_read_array(block.checked_add(PER_CAMERA_PROJECTION)?)?,
            near: try_read(block.checked_add(PER_CAMERA_NEAR)?)?,
            far: try_read(block.checked_add(PER_CAMERA_FAR)?)?,
            position: try_read_array(block.checked_add(PER_CAMERA_POSITION)?)?,
        })
    }
}

impl CameraController {
    /// The address of the function in the controller's vtable slot 9, `GetWorldTranslate` for the
    /// gameplay camera controller, or `None` when the vtable or the slot cannot be read.
    pub fn world_translate_fn(self) -> Option<GameAddr> {
        let vtable = try_read::<usize>(self.0)?;
        let slot = GameAddr::new(vtable).checked_add(CONTROLLER_WORLD_TRANSLATE_SLOT)?;
        try_read::<usize>(slot).map(GameAddr::new)
    }
}

#[cfg(test)]
mod tests {
    use std::hint::black_box;

    use game_mem::GameAddr;

    use super::{CameraController, RenderView};
    use crate::offsets::{
        CONTROLLER_WORLD_TRANSLATE_SLOT, PER_CAMERA_FAR, PER_CAMERA_NEAR, PER_CAMERA_POSITION,
        PER_CAMERA_PROJECTION, PER_CAMERA_VIEW, VIEW_PER_CAMERA, VIEW_READ_STATE_CONTROLLER,
    };

    /// Windows reserves the lowest 64 KiB of every process, so no read there succeeds.
    const UNMAPPED: GameAddr = GameAddr::new(0x10);

    /// A controller address that a test view names. No read goes there.
    const CONTROLLER: usize = 0x1234_5678;

    fn address_of(bytes: &[u8]) -> GameAddr {
        GameAddr::new(bytes.as_ptr().expose_provenance())
    }

    fn put(bytes: &mut [u8], offset: usize, value: &[u8]) {
        bytes[offset..][..value.len()].copy_from_slice(value);
    }

    fn floats(values: &[f32]) -> Vec<u8> {
        values.iter().flat_map(|value| value.to_ne_bytes()).collect()
    }

    #[test]
    fn a_view_yields_its_controller_and_its_per_camera_fields() {
        let mut view = vec![0_u8; VIEW_PER_CAMERA + 0x100];
        put(&mut view, VIEW_READ_STATE_CONTROLLER, &CONTROLLER.to_ne_bytes());
        let block = VIEW_PER_CAMERA;
        let matrix: Vec<f32> = (0_u8..16).map(f32::from).collect();
        let projection: Vec<f32> = (16_u8..32).map(f32::from).collect();
        put(&mut view, block + PER_CAMERA_VIEW, &floats(&matrix));
        put(&mut view, block + PER_CAMERA_PROJECTION, &floats(&projection));
        put(&mut view, block + PER_CAMERA_NEAR, &floats(&[0.5]));
        put(&mut view, block + PER_CAMERA_FAR, &floats(&[900.0]));
        put(&mut view, block + PER_CAMERA_POSITION, &floats(&[1.0, 2.0, 3.0]));
        let view = black_box(view);
        let render_view = RenderView::new(address_of(&view));
        let controller = render_view.read_state_controller();
        assert_eq!(controller, Some(CameraController(GameAddr::new(CONTROLLER))));
        let read = render_view.per_camera().expect("a readable block");
        assert_eq!(read.view.to_vec(), matrix);
        assert_eq!(read.projection.to_vec(), projection);
        assert_eq!((read.near, read.far), (0.5, 900.0));
        assert_eq!(read.position, [1.0, 2.0, 3.0]);
    }

    #[test]
    fn a_null_controller_pointer_and_an_unmapped_view_yield_nothing() {
        let view = black_box(vec![0_u8; VIEW_PER_CAMERA + 0x100]);
        assert_eq!(RenderView::new(address_of(&view)).read_state_controller(), None);
        assert_eq!(RenderView::new(UNMAPPED).read_state_controller(), None);
        assert_eq!(RenderView::new(UNMAPPED).per_camera(), None);
    }

    #[test]
    fn a_controller_yields_its_slot_9_function_and_an_unmapped_one_nothing() {
        let mut vtable = vec![0_usize; 12];
        vtable[CONTROLLER_WORLD_TRANSLATE_SLOT / size_of::<usize>()] = 0xABCD;
        let vtable = black_box(vtable);
        let object = black_box([vtable.as_ptr().expose_provenance()]);
        let controller = CameraController(GameAddr::new(object.as_ptr().expose_provenance()));
        assert_eq!(controller.world_translate_fn(), Some(GameAddr::new(0xABCD)));
        assert_eq!(CameraController(UNMAPPED).world_translate_fn(), None);
    }
}
