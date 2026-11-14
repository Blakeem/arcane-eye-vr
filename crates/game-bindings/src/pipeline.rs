//! Typed, guarded reads of the render pipeline, a `RenderFrame`, the level manager and an object's
//! vtable slot, and the guarded write of a render view's occlusion object.
//!
//! Each read goes through `game-mem`'s guarded copy, so a wrong address yields `None` and never a
//! fault. A read checks no type, so a right address is the caller's part.

use game_mem::{GameAddr, WriteDest, WriteError, try_read, try_write};

use crate::offsets::{
    FRAME_STAGE_RESOURCES, FRAME_VIEW_COUNT, FRAME_VIEW_LIST, LEVEL_ENVIRONMENT,
    LEVEL_ENVIRONMENT_IDS, LEVEL_MANAGER_LEVEL, LEVEL_SCENE, OCCLUSION_ENABLED, PIPELINE_MAIN_VIEW,
    PIPELINE_SPECIAL_VIEW, STAGE_RESOURCES_RECORD_COUNT, STAGE_RESOURCES_RECORDS,
};

/// The most entries a list check reads. A count above it reads as an unreadable list, since the
/// engine's lists hold a few views.
pub const LIST_LIMIT: u32 = 64;

/// The render pipeline, by its address in the game process.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pipeline(GameAddr);

/// The main view and the special view a pipeline holds. A null pointer reads as address 0.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PipelineViews {
    pub main: GameAddr,
    pub special: GameAddr,
}

/// A `RenderFrame`, by its address in the game process.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RenderFrame(GameAddr);

/// The level manager, by its address in the game process.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LevelManager(GameAddr);

/// The current level's scene and environment ids. A null pointer reads as address 0, and the ids
/// read as 0 when the level or its environment object is null.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Level {
    pub scene: GameAddr,
    pub environment_ids: GameAddr,
}

/// The occlusion object of a render view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Occlusion;

impl Pipeline {
    pub const fn new(address: GameAddr) -> Self {
        Self(address)
    }

    pub const fn address(self) -> GameAddr {
        self.0
    }

    /// The pipeline's main and special views, or `None` when either pointer cannot be read.
    pub fn views(self) -> Option<PipelineViews> {
        Some(PipelineViews {
            main: pointer(self.0, PIPELINE_MAIN_VIEW)?,
            special: pointer(self.0, PIPELINE_SPECIAL_VIEW)?,
        })
    }
}

impl RenderFrame {
    pub const fn new(address: GameAddr) -> Self {
        Self(address)
    }

    pub const fn address(self) -> GameAddr {
        self.0
    }

    /// The number of views in the frame's view list, or `None` when it cannot be read.
    pub fn view_count(self) -> Option<u32> {
        try_read::<u32>(self.0.checked_add(FRAME_VIEW_COUNT)?)
    }

    /// Whether the frame's view list holds `view`, or `None` when the list cannot be read or holds
    /// more than [`LIST_LIMIT`] views.
    pub fn lists_view(self, view: GameAddr) -> Option<bool> {
        let list = pointer(self.0, FRAME_VIEW_LIST)?;
        let count = self.view_count()?;
        any_entry(list, count, |entry| Some(entry == view))
    }

    /// Whether the frame's stage resource manager holds a record for `view`, or `None` when the
    /// record list cannot be read or holds more than [`LIST_LIMIT`] records.
    pub fn has_record(self, view: GameAddr) -> Option<bool> {
        let manager = pointer(self.0, FRAME_STAGE_RESOURCES)?;
        let records = pointer(manager, STAGE_RESOURCES_RECORDS)?;
        let count = try_read::<i32>(manager.checked_add(STAGE_RESOURCES_RECORD_COUNT)?)?;
        let count = u32::try_from(count).ok()?;
        any_entry(records, count, |record| Some(pointer(record, 0)? == view))
    }
}

impl LevelManager {
    pub const fn new(address: GameAddr) -> Self {
        Self(address)
    }

    /// The current level's scene and environment ids, or `None` when a pointer cannot be read.
    pub fn level(self) -> Option<Level> {
        let level = pointer(self.0, LEVEL_MANAGER_LEVEL)?;
        if level.get() == 0 {
            return Some(Level { scene: level, environment_ids: level });
        }
        let environment = pointer(level, LEVEL_ENVIRONMENT)?;
        let environment_ids = match environment.get() {
            0 => environment,
            _ => pointer(environment, LEVEL_ENVIRONMENT_IDS)?,
        };
        Some(Level { scene: pointer(level, LEVEL_SCENE)?, environment_ids })
    }
}

impl Occlusion {
    /// The bytes from an occlusion object's start that hold each field its writes fill.
    pub const WRITE_LEN: usize = OCCLUSION_ENABLED + size_of::<u8>();

    /// Sets the enabled byte of the occlusion object whose first [`Occlusion::WRITE_LEN`] bytes are
    /// `occlusion`.
    ///
    /// # Errors
    ///
    /// Returns the [`WriteError`] of a field that cannot be written or lies outside `occlusion`.
    pub fn write_enabled(occlusion: WriteDest<'_>, enabled: u8) -> Result<(), WriteError> {
        try_write(occlusion.offset(OCCLUSION_ENABLED)?, enabled)
    }
}

/// The function in the vtable slot `slot` bytes past the start of `object`'s vtable, or `None` when
/// the vtable pointer or the slot cannot be read.
pub fn vtable_slot(object: GameAddr, slot: usize) -> Option<GameAddr> {
    let vtable = pointer(object, 0)?;
    pointer(vtable, slot)
}

/// The pointer `offset` bytes past `address`, or `None` when it cannot be read.
fn pointer(address: GameAddr, offset: usize) -> Option<GameAddr> {
    try_read::<usize>(address.checked_add(offset)?).map(GameAddr::new)
}

/// Whether `matches` holds for one of the `count` pointers from `list` on, or `None` when a pointer
/// or a match cannot be read or `count` exceeds [`LIST_LIMIT`].
fn any_entry(
    list: GameAddr,
    count: u32,
    matches: impl Fn(GameAddr) -> Option<bool>,
) -> Option<bool> {
    if count > LIST_LIMIT {
        return None;
    }
    for index in 0..count {
        let offset = usize::try_from(index).ok()?.checked_mul(size_of::<usize>())?;
        if matches(pointer(list, offset)?)? {
            return Some(true);
        }
    }
    Some(false)
}

#[cfg(test)]
mod tests;
