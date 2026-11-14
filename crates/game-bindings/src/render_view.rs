//! Typed, guarded reads and writes of an engine render view and of the camera controller its
//! `ReadState` names.
//!
//! Each read and write goes through `game-mem`'s guarded copy, so a wrong address yields a failure
//! value and never a fault. A read checks no type, so a right address is the caller's part.

use game_mem::{GameAddr, WriteError, try_read, try_read_array, try_write};

use crate::camera::PerCamera;
use crate::offsets::{
    CONTROLLER_SIZE, CONTROLLER_VIEW, CONTROLLER_VTABLE_SLOTS, CONTROLLER_WORLD_TRANSLATE_SLOT,
    PER_CAMERA_FAR, PER_CAMERA_NEAR, PER_CAMERA_POSITION, PER_CAMERA_PROJECTION, PER_CAMERA_VIEW,
    VIEW_LAST_SEEN_CONTROLLER, VIEW_PER_CAMERA, VIEW_READ_STATE_CONTROLLER,
};

/// A render view, by its address in the game process.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RenderView(GameAddr);

/// A camera controller, by its address in the game process.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CameraController(GameAddr);

/// What a render view's `ReadState` controller pointer holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadState {
    /// A null pointer, as the special view holds.
    Null,
    Controller(CameraController),
}

/// A byte copy of a camera controller object, taken in one guarded read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ControllerCopy([u8; CONTROLLER_SIZE]);

/// The index of the view matrix's first `f32` in a controller's bytes.
const VIEW_WORD: usize = CONTROLLER_VIEW / size_of::<f32>();

impl RenderView {
    pub const fn new(address: GameAddr) -> Self {
        Self(address)
    }

    pub const fn address(self) -> GameAddr {
        self.0
    }

    /// What the view's `ReadState` controller pointer holds, or `None` when it cannot be read.
    pub fn read_state(self) -> Option<ReadState> {
        let pointer = try_read::<usize>(self.0.checked_add(VIEW_READ_STATE_CONTROLLER)?)?;
        let state = match pointer {
            0 => ReadState::Null,
            _ => ReadState::Controller(CameraController(GameAddr::new(pointer))),
        };
        Some(state)
    }

    /// The camera controller the view's `ReadState` points to, or `None` when that pointer is null or
    /// cannot be read.
    pub fn read_state_controller(self) -> Option<CameraController> {
        match self.read_state()? {
            ReadState::Controller(controller) => Some(controller),
            ReadState::Null => None,
        }
    }

    /// Points the view's `ReadState` at `controller`. The engine's frame sync points it back at the
    /// `WriteState` controller at the frame's end.
    ///
    /// # Errors
    ///
    /// Returns the [`WriteError`] of a field that cannot be written.
    pub fn write_read_state(self, controller: CameraController) -> Result<(), WriteError> {
        self.write_pointer(VIEW_READ_STATE_CONTROLLER, controller)
    }

    /// Sets the controller pointer the view saw last to `controller`. Begin-recording then sets no
    /// camera cut when the `ReadState` controller is `controller`.
    ///
    /// # Errors
    ///
    /// Returns the [`WriteError`] of a field that cannot be written.
    pub fn write_last_seen_controller(
        self,
        controller: CameraController,
    ) -> Result<(), WriteError> {
        self.write_pointer(VIEW_LAST_SEEN_CONTROLLER, controller)
    }

    /// The controller pointer the view saw last, null included, or `None` when it cannot be read.
    pub fn last_seen_controller(self) -> Option<GameAddr> {
        try_read::<usize>(self.0.checked_add(VIEW_LAST_SEEN_CONTROLLER)?).map(GameAddr::new)
    }

    fn write_pointer(self, offset: usize, controller: CameraController) -> Result<(), WriteError> {
        try_write(self.0.destination(offset)?, controller.0.get())
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
    pub const fn new(address: GameAddr) -> Self {
        Self(address)
    }

    pub const fn address(self) -> GameAddr {
        self.0
    }

    /// The address of the function in the controller's vtable slot 9, `GetWorldTranslate` for the
    /// gameplay camera controller, or `None` when the vtable or the slot cannot be read.
    pub fn world_translate_fn(self) -> Option<GameAddr> {
        let vtable = try_read::<usize>(self.0)?;
        let slot = GameAddr::new(vtable).checked_add(CONTROLLER_WORLD_TRANSLATE_SLOT)?;
        try_read::<usize>(slot).map(GameAddr::new)
    }

    /// A copy of the controller object's bytes, or `None` when any of them cannot be read.
    pub fn copy(self) -> Option<ControllerCopy> {
        try_read_array(self.0).map(ControllerCopy)
    }

    /// The first [`CONTROLLER_VTABLE_SLOTS`] slots of the controller's vtable, or `None` when the
    /// vtable pointer or any slot cannot be read.
    pub fn vtable_slots(self) -> Option<[usize; CONTROLLER_VTABLE_SLOTS]> {
        let vtable = try_read::<usize>(self.0)?;
        try_read_array(GameAddr::new(vtable))
    }
}

impl ControllerCopy {
    pub const fn bytes(&self) -> &[u8; CONTROLLER_SIZE] {
        &self.0
    }

    /// The copied world-to-camera view matrix, 16 `f32` in column-major order.
    pub fn view(&self) -> [f32; 16] {
        let (words, _) = self.0.as_chunks::<4>();
        let mut view = [0.0; 16];
        for (value, word) in view.iter_mut().zip(words.iter().skip(VIEW_WORD)) {
            *value = f32::from_ne_bytes(*word);
        }
        view
    }
}

#[cfg(test)]
mod tests {
    use std::hint::black_box;

    use game_mem::{GameAddr, WriteError, try_read};

    use super::{CameraController, ReadState, RenderView};
    use crate::offsets::{
        CONTROLLER_SIZE, CONTROLLER_VIEW, CONTROLLER_VTABLE_SLOTS, CONTROLLER_WORLD_TRANSLATE_SLOT,
        PER_CAMERA_FAR, PER_CAMERA_NEAR, PER_CAMERA_POSITION, PER_CAMERA_PROJECTION,
        PER_CAMERA_VIEW, VIEW_LAST_SEEN_CONTROLLER, VIEW_PER_CAMERA, VIEW_READ_STATE_CONTROLLER,
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

    #[test]
    fn read_state_tells_a_null_pointer_from_an_unreadable_one() {
        let mut view = vec![0_u8; VIEW_PER_CAMERA];
        let null = RenderView::new(address_of(&view)).read_state();
        put(&mut view, VIEW_READ_STATE_CONTROLLER, &CONTROLLER.to_ne_bytes());
        let view = black_box(view);
        let set = RenderView::new(address_of(&view)).read_state();
        assert_eq!(null, Some(ReadState::Null));
        let named = CameraController::new(GameAddr::new(CONTROLLER));
        assert_eq!(set, Some(ReadState::Controller(named)));
        assert_eq!(named.address(), GameAddr::new(CONTROLLER));
        assert_eq!(RenderView::new(UNMAPPED).read_state(), None);
    }

    #[test]
    fn the_pointer_writes_land_in_the_read_state_and_last_seen_fields() {
        let mut view = black_box(vec![0_u8; VIEW_LAST_SEEN_CONTROLLER + 8]);
        let address = GameAddr::new(view.as_mut_ptr().expose_provenance());
        let render_view = RenderView::new(address);
        let first = CameraController::new(GameAddr::new(CONTROLLER));
        let second = CameraController::new(GameAddr::new(CONTROLLER + 0x40));
        assert_eq!(render_view.write_read_state(first), Ok(()));
        assert_eq!(render_view.write_last_seen_controller(second), Ok(()));
        let field = |offset| try_read::<usize>(address.checked_add(offset).expect("a field"));
        assert_eq!(field(VIEW_READ_STATE_CONTROLLER), Some(CONTROLLER));
        assert_eq!(field(VIEW_LAST_SEEN_CONTROLLER), Some(CONTROLLER + 0x40));
        assert_eq!(render_view.read_state_controller(), Some(first));
        assert_eq!(render_view.last_seen_controller(), Some(GameAddr::new(CONTROLLER + 0x40)));
        assert_eq!(render_view.address(), address);
        drop(black_box(view));
    }

    #[test]
    fn the_last_seen_controller_reads_a_null_pointer_and_nothing_from_an_unmapped_view() {
        let view = black_box(vec![0_u8; VIEW_LAST_SEEN_CONTROLLER + 8]);
        let last_seen = RenderView::new(address_of(&view)).last_seen_controller();
        assert_eq!(last_seen, Some(GameAddr::new(0)));
        assert_eq!(RenderView::new(UNMAPPED).last_seen_controller(), None);
    }

    #[test]
    fn a_pointer_write_to_an_unmapped_or_overflowing_view_yields_the_failure() {
        let controller = CameraController::new(GameAddr::new(CONTROLLER));
        let unmapped = RenderView::new(UNMAPPED).write_read_state(controller);
        let field = UNMAPPED.get() + VIEW_READ_STATE_CONTROLLER;
        assert_eq!(unmapped, Err(WriteError::Faulted { address: field }));
        let top = RenderView::new(GameAddr::new(usize::MAX));
        assert_eq!(top.write_last_seen_controller(controller), Err(WriteError::PastAddressSpace));
    }

    #[test]
    fn a_controller_copy_holds_its_bytes_and_view_matrix_and_the_vtable_its_slots() {
        let slots: Vec<usize> = (1..=CONTROLLER_VTABLE_SLOTS).collect();
        let slots = black_box(slots);
        let matrix: Vec<f32> = (0_u8..16).map(|value| f32::from(value) + 0.5).collect();
        let mut object = vec![0x5A_u8; CONTROLLER_SIZE];
        put(&mut object, 0, &slots.as_ptr().expose_provenance().to_ne_bytes());
        put(&mut object, CONTROLLER_VIEW, &floats(&matrix));
        let object = black_box(object);
        let controller = CameraController::new(address_of(&object));
        let copy = controller.copy().expect("a readable controller");
        assert_eq!(copy.bytes().as_slice(), object.as_slice());
        assert_eq!(copy.view().to_vec(), matrix);
        let read = controller.vtable_slots().map(|read| read.to_vec());
        assert_eq!(read.as_ref(), Some(&slots));
        assert_eq!(CameraController::new(UNMAPPED).copy(), None);
        assert_eq!(CameraController::new(UNMAPPED).vtable_slots(), None);
    }
}
