//! Shadow stage reads against a stage built in the test process's memory.

use std::hint::black_box;

use game_mem::GameAddr;

use super::{ShadowCache, ShadowStage};
use crate::offsets::{
    SHADOW_CACHE_ON, SHADOW_CACHE_PARITY, SHADOW_CACHE_RECORDS, SHADOW_CACHE_RECORDS_SIZE,
    SHADOW_STAGE_VIEW,
};
use crate::render_view::RenderView;

/// Windows reserves the lowest 64 KiB of every process, so no read there succeeds.
const UNMAPPED: GameAddr = GameAddr::new(0x10);

/// A view address that the test stage names. No read goes there.
const VIEW: usize = 0x1234_5678;

#[test]
fn a_stage_yields_its_view_and_its_cache_state() {
    let mut bytes: Vec<u8> = (0..=u8::MAX).cycle().take(ShadowStage::WRITE_LEN).collect();
    bytes[SHADOW_STAGE_VIEW..][..8].copy_from_slice(&VIEW.to_ne_bytes());
    bytes[SHADOW_CACHE_PARITY] = 1;
    bytes[SHADOW_CACHE_ON] = 0xA5;
    let bytes = black_box(bytes);
    let stage = ShadowStage::new(GameAddr::new(bytes.as_ptr().expose_provenance()));
    assert_eq!(stage.view(), Some(RenderView::new(GameAddr::new(VIEW))));
    let mut records = [0_u8; SHADOW_CACHE_RECORDS_SIZE];
    records.copy_from_slice(&bytes[SHADOW_CACHE_RECORDS..][..SHADOW_CACHE_RECORDS_SIZE]);
    let expected = ShadowCache { parity: 1, records, on: 0xA5 };
    assert_eq!(stage.cache(), Some(expected));
}

#[test]
fn an_unmapped_stage_yields_no_view_and_no_cache_state() {
    assert_eq!(ShadowStage::new(UNMAPPED).view(), None);
    assert_eq!(ShadowStage::new(UNMAPPED).cache(), None);
}
