//! Render views and camera controllers built in the test process's memory, for the view camera's
//! reads and the eye camera's reads and writes.

use std::hint::black_box;

use game_bindings::camera::PerCamera;
use game_bindings::offsets::{
    CONTROLLER_WORLD_TRANSLATE_SLOT, PER_CAMERA_FAR, PER_CAMERA_NEAR, PER_CAMERA_POSITION,
    PER_CAMERA_PROJECTION, PER_CAMERA_VIEW, VIEW_LAST_SEEN_CONTROLLER, VIEW_PER_CAMERA,
    VIEW_READ_STATE_CONTROLLER,
};
use game_bindings::render_view::RenderView;
use game_mem::GameAddr;

/// The address a test gives the resolved `GetWorldTranslate`. No code runs there.
pub(crate) const WORLD_TRANSLATE: GameAddr = GameAddr::new(0x7FF6_0000_1000);

/// A camera controller object whose vtable holds `slot_9` in slot 9.
pub(crate) struct TestController {
    _vtable: Box<[usize]>,
    object: Box<[usize; 1]>,
}

impl TestController {
    pub(crate) fn new(slot_9: GameAddr) -> Self {
        let slot = CONTROLLER_WORLD_TRANSLATE_SLOT.checked_div(size_of::<usize>()).expect("a slot");
        let mut vtable = vec![0; 16].into_boxed_slice();
        vtable[slot] = slot_9.get();
        let object = Box::new([vtable.as_ptr().expose_provenance()]);
        black_box(Self { _vtable: vtable, object })
    }

    pub(crate) fn address(&self) -> usize {
        self.object.as_ptr().expose_provenance()
    }
}

/// A render view whose `ReadState` controller pointer is `controller` and whose `PerCamera` block 0
/// holds `block`. The eye camera writes its `ReadState` and last-seen controller fields.
pub(crate) struct TestView {
    _bytes: Box<[u8]>,
    /// The address of the bytes, taken from a mutable pointer, since the eye camera writes there.
    address: GameAddr,
}

impl TestView {
    pub(crate) fn new(controller: usize, block: &PerCamera) -> Self {
        let mut bytes = vec![0_u8; VIEW_LAST_SEEN_CONTROLLER + 8].into_boxed_slice();
        put(&mut bytes, VIEW_READ_STATE_CONTROLLER, &controller.to_ne_bytes());
        put(&mut bytes, VIEW_PER_CAMERA + PER_CAMERA_VIEW, &floats(&block.view));
        put(&mut bytes, VIEW_PER_CAMERA + PER_CAMERA_PROJECTION, &floats(&block.projection));
        put(&mut bytes, VIEW_PER_CAMERA + PER_CAMERA_NEAR, &block.near.to_ne_bytes());
        put(&mut bytes, VIEW_PER_CAMERA + PER_CAMERA_FAR, &block.far.to_ne_bytes());
        put(&mut bytes, VIEW_PER_CAMERA + PER_CAMERA_POSITION, &floats(&block.position));
        let address = GameAddr::new(bytes.as_mut_ptr().expose_provenance());
        black_box(Self { _bytes: bytes, address })
    }

    pub(crate) const fn view(&self) -> RenderView {
        RenderView::new(self.address)
    }

    /// The pointer in the field `offset` bytes into the view.
    pub(crate) fn pointer_at(&self, offset: usize) -> usize {
        let field = self.address.checked_add(offset).expect("a view field");
        game_mem::try_read(field).expect("a readable view field")
    }

    /// Does what the engine does to the view after the begin-recording hook: begin-recording stores
    /// the `ReadState` pointer as the last-seen controller, and the frame sync at the frame's end
    /// points the `ReadState` at `controller`.
    pub(crate) fn finish_frame(&self, controller: usize) {
        let read_state = self.pointer_at(VIEW_READ_STATE_CONTROLLER);
        self.write_pointer(VIEW_LAST_SEEN_CONTROLLER, read_state);
        self.write_pointer(VIEW_READ_STATE_CONTROLLER, controller);
    }

    fn write_pointer(&self, offset: usize, value: usize) {
        let field = self.address.checked_add(offset).expect("a view field");
        game_mem::try_write(field, value).expect("a writable view field");
    }
}

/// A block with a 90-degree square perspective projection and its camera at `x` on the X axis, looking
/// along +Z.
pub(crate) fn block_at(x: f32) -> PerCamera {
    let mut view = [0.0; 16];
    for diagonal in [0, 5, 10, 15] {
        view[diagonal] = 1.0;
    }
    view[12] = -x;
    let mut projection = [0.0; 16];
    projection[0] = 1.0;
    projection[5] = 1.0;
    projection[10] = -0.0001;
    projection[11] = 1.0;
    projection[14] = 0.1;
    PerCamera { view, projection, near: 0.1, far: 1000.0, position: [x, 0.0, 0.0] }
}

fn put(bytes: &mut [u8], offset: usize, value: &[u8]) {
    bytes[offset..][..value.len()].copy_from_slice(value);
}

fn floats(values: &[f32]) -> Vec<u8> {
    values.iter().flat_map(|value| value.to_ne_bytes()).collect()
}
