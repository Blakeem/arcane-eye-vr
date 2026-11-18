//! Typed, guarded reads and writes of the composite stage, the stage of the pipeline's special view
//! that draws the finished scene and the UI texture into the back buffer.
//!
//! The stage's job draws the UI texture only while the stage's UI enable byte is nonzero. The stage
//! that renders the UI fills the UI texture whether or not the byte is set (a write of 0 in the game
//! and a capture of the UI texture after it).
//!
//! Each read and write goes through `game-mem`'s guarded copy, so a wrong address yields a failure
//! value and never a fault. A write goes to the stage's [`WriteDest`], which the caller's unsafe
//! code constructs.

use game_mem::{GameAddr, WriteDest, WriteError, try_read, try_write};

use crate::offsets::{COMPOSITE_STAGE_UI_ENABLED, VIEW_STAGE_ARRAY, VIEW_STAGE_COUNT};
use crate::render_view::RenderView;

/// The most stages of a view that a lookup reads. The main view held 29 stages and the special view
/// 3 (a dump in the game), so a larger count is not a stage count.
const MAX_STAGES: u32 = 64;

/// A composite stage, by its address in the game process.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompositeStage(GameAddr);

impl CompositeStage {
    /// The bytes from a stage's start that hold its UI enable byte.
    pub const WRITE_LEN: usize = COMPOSITE_STAGE_UI_ENABLED + size_of::<u8>();

    pub const fn new(address: GameAddr) -> Self {
        Self(address)
    }

    pub const fn address(self) -> GameAddr {
        self.0
    }

    /// The stage of `view`'s stage array whose first word is `vtable`, the composite stage's vtable.
    /// Returns `None` when the array holds no such stage, holds more than 64 stages or cannot be
    /// read.
    pub fn of(view: RenderView, vtable: GameAddr) -> Option<Self> {
        let count = try_read::<u32>(view.address().checked_add(VIEW_STAGE_COUNT)?)?;
        if count > MAX_STAGES {
            return None;
        }
        let array = try_read::<usize>(view.address().checked_add(VIEW_STAGE_ARRAY)?)?;
        let mut entry = GameAddr::new(array);
        for _ in 0..count {
            let stage = GameAddr::new(try_read::<usize>(entry)?);
            if stage.get() != 0 && try_read::<usize>(stage)? == vtable.get() {
                return Some(Self(stage));
            }
            entry = entry.checked_add(size_of::<usize>())?;
        }
        None
    }

    /// The stage's UI enable byte, or `None` when it cannot be read.
    pub fn ui_enabled(self) -> Option<u8> {
        try_read::<u8>(self.0.checked_add(COMPOSITE_STAGE_UI_ENABLED)?)
    }

    /// Sets the UI enable byte of the stage whose first [`CompositeStage::WRITE_LEN`] bytes are
    /// `stage`. The stage's job of the next frame reads it.
    ///
    /// # Errors
    ///
    /// Returns the [`WriteError`] of a byte that cannot be written or lies outside `stage`.
    pub fn write_ui_enabled(stage: WriteDest<'_>, enabled: u8) -> Result<(), WriteError> {
        try_write(stage.offset(COMPOSITE_STAGE_UI_ENABLED)?, enabled)
    }
}

#[cfg(test)]
mod tests;
