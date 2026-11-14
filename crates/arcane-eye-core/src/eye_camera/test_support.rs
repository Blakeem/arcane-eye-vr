//! Head camera controllers built in the test process's memory and eye cameras over fresh pages.

use std::hint::black_box;

use game_bindings::offsets::{
    CONTROLLER_SIZE, CONTROLLER_VIEW, CONTROLLER_VTABLE_SLOTS, CONTROLLER_WORLD_TRANSLATE_SLOT,
};
use game_mem::GameAddr;
use stereo_core::eye::Eye;

use super::EyeState;
use super::proxy::RING_SIZE;
use crate::raw::SharedPages;
use crate::view_camera::test_support::WORLD_TRANSLATE;

/// The main view's head view matrix measured on the title screen, in memory order.
pub(crate) const HEAD_VIEW: [f32; 16] = [
    0.131_419_54,
    0.356_776_65,
    -0.924_899_8,
    0.0,
    0.007_081_553_3,
    0.932_629_94,
    0.360_764_8,
    0.0,
    0.991_301_8,
    -0.053_961_32,
    0.120_039_22,
    0.0,
    943.181_46,
    -286.244_78,
    1_105.491_7,
    1.0,
];

/// The world position measured with `HEAD_VIEW`.
pub(crate) const HEAD_POSITION: [f32; 3] = [1_000.641_7, -138.541_24, -1_083.125_5];

/// The right axis of `HEAD_VIEW`: elements 0, 4 and 8.
pub(crate) const RIGHT: [f32; 3] = [0.131_419_54, 0.007_081_553_3, 0.991_301_8];

/// Half of a 63 mm distance between the eyes, in meters.
pub(crate) const HALF_IPD_M: f32 = 0.0315;

/// A gameplay camera controller: its bytes with `HEAD_VIEW` at the view matrix and a vtable whose
/// slots all differ, with `slot_9` in slot 9.
pub(crate) struct HeadController {
    vtable: Box<[usize]>,
    object: Box<[u8]>,
}

impl HeadController {
    pub(crate) fn new(slot_9: GameAddr) -> Self {
        let slot = CONTROLLER_WORLD_TRANSLATE_SLOT.checked_div(size_of::<usize>()).expect("a slot");
        let mut vtable: Box<[usize]> =
            (0..CONTROLLER_VTABLE_SLOTS).map(|n| n.wrapping_add(0x1000)).collect();
        vtable[slot] = slot_9.get();
        let mut object: Box<[u8]> = (0..=u8::MAX).cycle().take(CONTROLLER_SIZE).collect();
        object[..8].copy_from_slice(&vtable.as_ptr().expose_provenance().to_ne_bytes());
        let view: Vec<u8> = HEAD_VIEW.iter().flat_map(|value| value.to_ne_bytes()).collect();
        object[CONTROLLER_VIEW..][..view.len()].copy_from_slice(&view);
        black_box(Self { vtable, object })
    }

    pub(crate) fn address(&self) -> GameAddr {
        GameAddr::new(self.object.as_ptr().expose_provenance())
    }

    pub(crate) fn bytes(&self) -> &[u8] {
        &self.object
    }

    pub(crate) fn vtable(&self) -> &[usize] {
        &self.vtable
    }
}

/// The eye camera's state for `eye`, half a 63 mm distance from the head, over fresh pages, whose
/// head controllers have `WORLD_TRANSLATE` in slot 9.
pub(crate) fn eye_state(eye: Eye) -> EyeState {
    let pages = SharedPages::commit(RING_SIZE).expect("commit the proxy pages");
    let offset_m = match eye {
        Eye::Left => -HALF_IPD_M,
        Eye::Right => HALF_IPD_M,
    };
    EyeState::new(pages, WORLD_TRANSLATE, eye, offset_m)
}
