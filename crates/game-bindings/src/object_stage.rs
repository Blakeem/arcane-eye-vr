//! Typed, guarded reads and writes of a render view's object data stage and of its slot index.
//!
//! Each read and write goes through `game-mem`'s guarded copy, so a wrong address yields a failure
//! value and never a fault. A read checks no type, so a right address is the caller's part. A write
//! goes to the stage's [`WriteDest`], which the caller's unsafe code constructs.

use game_mem::{GameAddr, WriteDest, WriteError, try_read, try_write};

use crate::offsets::{OBJECT_STAGE_SLOT, OBJECT_STAGE_VIEW};
use crate::render_view::RenderView;

/// A render view's object data stage, by its address in the game process.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ObjectStage(GameAddr);

impl ObjectStage {
    /// The bytes from a stage's start that hold its slot index.
    pub const WRITE_LEN: usize = OBJECT_STAGE_SLOT + size_of::<i32>();

    pub const fn new(address: GameAddr) -> Self {
        Self(address)
    }

    pub const fn address(self) -> GameAddr {
        self.0
    }

    /// The stage's render view, address 0 included, or `None` when the pointer cannot be read.
    pub fn view(self) -> Option<RenderView> {
        let view = try_read::<usize>(self.0.checked_add(OBJECT_STAGE_VIEW)?)?;
        Some(RenderView::new(GameAddr::new(view)))
    }

    /// The stage's slot index, or `None` when it cannot be read.
    pub fn slot(self) -> Option<i32> {
        try_read::<i32>(self.0.checked_add(OBJECT_STAGE_SLOT)?)
    }

    /// Sets the slot index of the stage whose first [`ObjectStage::WRITE_LEN`] bytes are `stage` to
    /// `slot`. Every other byte keeps its value.
    ///
    /// # Errors
    ///
    /// Returns the [`WriteError`] of a slot index that cannot be written or lies outside `stage`.
    pub fn write_slot(stage: WriteDest<'_>, slot: i32) -> Result<(), WriteError> {
        try_write(stage.offset(OBJECT_STAGE_SLOT)?, slot)
    }
}

#[cfg(test)]
mod tests;
