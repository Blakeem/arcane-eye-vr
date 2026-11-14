//! Render views and camera controllers built in the test process's memory, for the view camera's reads.

use std::hint::black_box;

use game_bindings::camera::PerCamera;
use game_bindings::offsets::{
    CONTROLLER_WORLD_TRANSLATE_SLOT, PER_CAMERA_FAR, PER_CAMERA_NEAR, PER_CAMERA_POSITION,
    PER_CAMERA_PROJECTION, PER_CAMERA_VIEW, VIEW_PER_CAMERA, VIEW_READ_STATE_CONTROLLER,
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
/// holds `block`.
pub(crate) struct TestView {
    bytes: Box<[u8]>,
}

impl TestView {
    pub(crate) fn new(controller: usize, block: &PerCamera) -> Self {
        let mut bytes = vec![0_u8; VIEW_PER_CAMERA + 0x100].into_boxed_slice();
        put(&mut bytes, VIEW_READ_STATE_CONTROLLER, &controller.to_ne_bytes());
        put(&mut bytes, VIEW_PER_CAMERA + PER_CAMERA_VIEW, &floats(&block.view));
        put(&mut bytes, VIEW_PER_CAMERA + PER_CAMERA_PROJECTION, &floats(&block.projection));
        put(&mut bytes, VIEW_PER_CAMERA + PER_CAMERA_NEAR, &block.near.to_ne_bytes());
        put(&mut bytes, VIEW_PER_CAMERA + PER_CAMERA_FAR, &block.far.to_ne_bytes());
        put(&mut bytes, VIEW_PER_CAMERA + PER_CAMERA_POSITION, &floats(&block.position));
        black_box(Self { bytes })
    }

    pub(crate) fn view(&self) -> RenderView {
        RenderView::new(GameAddr::new(self.bytes.as_ptr().expose_provenance()))
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
