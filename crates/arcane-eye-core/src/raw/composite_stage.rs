//! The write destination of the composite stage's UI enable byte.

use game_bindings::composite_stage::CompositeStage;
use game_mem::WriteDest;

/// The destination of guarded writes to the first [`CompositeStage::WRITE_LEN`] bytes of `stage`.
pub(crate) fn stage_writes<'stage>(stage: CompositeStage) -> WriteDest<'stage> {
    // SAFETY: the UI layer passes only a stage address it read from the special view's stage array.
    // Those bytes are game memory or unmapped, and no Rust reference covers them.
    unsafe { WriteDest::new(stage.address(), CompositeStage::WRITE_LEN) }
}
