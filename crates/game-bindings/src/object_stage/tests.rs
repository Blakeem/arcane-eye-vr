//! Object data stage reads against a stage built in the test process's memory.

use std::hint::black_box;

use game_mem::GameAddr;

use super::ObjectStage;
use crate::offsets::{OBJECT_STAGE_SLOT, OBJECT_STAGE_VIEW};
use crate::render_view::RenderView;

/// Windows reserves the lowest 64 KiB of every process, so no read there succeeds.
const UNMAPPED: GameAddr = GameAddr::new(0x10);

/// A view address that the test stage names. No read goes there.
const VIEW: usize = 0x1234_5678;

#[test]
fn a_stage_yields_its_view_and_its_slot_index() {
    let mut bytes = vec![0xEE_u8; ObjectStage::WRITE_LEN];
    bytes[OBJECT_STAGE_VIEW..][..8].copy_from_slice(&VIEW.to_ne_bytes());
    bytes[OBJECT_STAGE_SLOT..][..4].copy_from_slice(&7_i32.to_ne_bytes());
    let bytes = black_box(bytes);
    let stage = ObjectStage::new(GameAddr::new(bytes.as_ptr().expose_provenance()));
    assert_eq!(stage.view(), Some(RenderView::new(GameAddr::new(VIEW))));
    assert_eq!(stage.slot(), Some(7));
}

#[test]
fn an_unmapped_stage_yields_no_view_and_no_slot_index() {
    assert_eq!(ObjectStage::new(UNMAPPED).view(), None);
    assert_eq!(ObjectStage::new(UNMAPPED).slot(), None);
}
