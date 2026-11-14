//! Typed, guarded reads and writes of a render view's shadow stage and of its distant shadow cache
//! state, which the cache's readers take.
//!
//! Each read and write goes through `game-mem`'s guarded copy, so a wrong address yields a failure
//! value and never a fault. A read checks no type, so a right address is the caller's part. A write
//! goes to the stage's [`WriteDest`], which the caller's unsafe code constructs.

use game_mem::{
    GameAddr, WriteDest, WriteError, try_read, try_read_array, try_write, try_write_slice,
};

use crate::offsets::{
    SHADOW_CACHE_ON, SHADOW_CACHE_PARITY, SHADOW_CACHE_RECORDS, SHADOW_CACHE_RECORDS_SIZE,
    SHADOW_STAGE_VIEW,
};
use crate::render_view::RenderView;

/// A render view's shadow stage, by its address in the game process.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShadowStage(GameAddr);

/// The state of a shadow stage's distant shadow cache that the cache's readers take.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShadowCache {
    pub parity: u8,
    /// The cache records, in world space, so they are valid for every view.
    pub records: [u8; SHADOW_CACHE_RECORDS_SIZE],
    pub on: u8,
}

// `WRITE_LEN` ends at the on byte, so every other written field must come before it.
const _: () = assert!(SHADOW_CACHE_PARITY < SHADOW_CACHE_RECORDS);
const _: () = assert!(SHADOW_CACHE_RECORDS + SHADOW_CACHE_RECORDS_SIZE <= SHADOW_CACHE_ON);

impl ShadowStage {
    /// The bytes from a stage's start that hold each field of its distant shadow cache state.
    pub const WRITE_LEN: usize = SHADOW_CACHE_ON + size_of::<u8>();

    pub const fn new(address: GameAddr) -> Self {
        Self(address)
    }

    pub const fn address(self) -> GameAddr {
        self.0
    }

    /// The stage's render view, address 0 included, or `None` when the pointer cannot be read.
    pub fn view(self) -> Option<RenderView> {
        let view = try_read::<usize>(self.0.checked_add(SHADOW_STAGE_VIEW)?)?;
        Some(RenderView::new(GameAddr::new(view)))
    }

    /// The stage's distant shadow cache state, or `None` when any field of it cannot be read.
    pub fn cache(self) -> Option<ShadowCache> {
        Some(ShadowCache {
            parity: try_read::<u8>(self.0.checked_add(SHADOW_CACHE_PARITY)?)?,
            records: try_read_array(self.0.checked_add(SHADOW_CACHE_RECORDS)?)?,
            on: try_read::<u8>(self.0.checked_add(SHADOW_CACHE_ON)?)?,
        })
    }

    /// Writes `cache` to the distant shadow cache state of the stage whose first
    /// [`ShadowStage::WRITE_LEN`] bytes are `stage`: the records, then the parity byte, then the on
    /// byte. Every other byte keeps its value.
    ///
    /// # Errors
    ///
    /// Returns the [`WriteError`] of a field that cannot be written or lies outside `stage`. The
    /// fields before it may then hold the new values.
    pub fn write_cache(stage: WriteDest<'_>, cache: &ShadowCache) -> Result<(), WriteError> {
        try_write_slice(stage.offset(SHADOW_CACHE_RECORDS)?, &cache.records)?;
        try_write(stage.offset(SHADOW_CACHE_PARITY)?, cache.parity)?;
        try_write(stage.offset(SHADOW_CACHE_ON)?, cache.on)
    }
}

#[cfg(test)]
mod tests;
