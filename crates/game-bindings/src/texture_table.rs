//! Typed, guarded reads of a pipeline's texture table, which maps a key to the Direct3D texture a
//! frame's stages render into.
//!
//! Each read goes through `game-mem`'s guarded copy, so a wrong address yields `None` and never a
//! fault. A read checks no type, so the caller checks a texture pointer before it calls through it.
//! The game gives a key another texture from frame to frame, so a caller looks a key up in every
//! frame and keeps no texture across frames.

use game_mem::{GameAddr, try_read};

use crate::offsets::{
    TEXTURE_NODE_RECORD, TEXTURE_RECORD_TEXTURE, TEXTURE_TABLE_BUCKET_COUNT, TEXTURE_TABLE_BUCKETS,
    TEXTURE_TABLE_ENTRY_COUNT, TEXTURE_TABLE_KEYS, TEXTURE_TABLE_NEXT, TEXTURE_TABLE_VALUES,
    VIEW_TEXTURE_TABLE,
};
use crate::render_view::RenderView;

/// The key of one texture of a frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextureKey(pub u32);

impl TextureKey {
    /// The game's entire UI, with premultiplied alpha.
    pub const UI: Self = Self(0x0C41_9ABC);

    /// The finished scene of every view, which the game's composite draws under the UI.
    pub const SCENE: Self = Self(0x684E_E890);
}

/// A pipeline's texture table, by its address in the game process.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextureTable(GameAddr);

/// The arrays of a table, read once per lookup.
#[derive(Clone, Copy)]
struct Arrays {
    bucket_count: u32,
    entry_count: usize,
    buckets: GameAddr,
    next: GameAddr,
    keys: GameAddr,
    values: GameAddr,
}

impl TextureTable {
    pub const fn new(address: GameAddr) -> Self {
        Self(address)
    }

    /// The texture table of `view`, or `None` when the view holds none or cannot be read. Every view
    /// of a pipeline holds the same table.
    pub fn of(view: RenderView) -> Option<Self> {
        let table = pointer(view.address(), VIEW_TEXTURE_TABLE)?;
        (table.get() != 0).then_some(Self(table))
    }

    /// The Direct3D texture that `key` names in this frame, or `None` when the table lacks the key,
    /// the key has no texture or a read fails.
    pub fn texture(self, key: TextureKey) -> Option<GameAddr> {
        let node = self.node(key)?;
        let record = non_null(pointer(node, TEXTURE_NODE_RECORD)?)?;
        non_null(pointer(record, TEXTURE_RECORD_TEXTURE)?)
    }

    /// The node of `key`: the value of the entry in the key's bucket chain whose key is `key`.
    fn node(self, key: TextureKey) -> Option<GameAddr> {
        let arrays = self.arrays()?;
        let bucket = usize::try_from(key.0.checked_rem(arrays.bucket_count)?).ok()?;
        let mut index = try_read::<i32>(element(arrays.buckets, bucket, size_of::<i32>())?)?;
        // A chain holds each entry at most once, so more steps than entries mean a broken chain.
        for _ in 0..arrays.entry_count {
            let entry = usize::try_from(index).ok().filter(|entry| *entry < arrays.entry_count)?;
            if try_read::<u32>(element(arrays.keys, entry, size_of::<u32>())?)? == key.0 {
                return non_null(pointer(arrays.values, entry.checked_mul(size_of::<usize>())?)?);
            }
            index = try_read::<i32>(element(arrays.next, entry, size_of::<i32>())?)?;
        }
        None
    }

    fn arrays(self) -> Option<Arrays> {
        let bucket_count = try_read::<i32>(self.0.checked_add(TEXTURE_TABLE_BUCKET_COUNT)?)?;
        let entry_count = try_read::<i32>(self.0.checked_add(TEXTURE_TABLE_ENTRY_COUNT)?)?;
        Some(Arrays {
            bucket_count: u32::try_from(bucket_count).ok()?,
            entry_count: usize::try_from(entry_count).ok()?,
            buckets: pointer(self.0, TEXTURE_TABLE_BUCKETS)?,
            next: pointer(self.0, TEXTURE_TABLE_NEXT)?,
            keys: pointer(self.0, TEXTURE_TABLE_KEYS)?,
            values: pointer(self.0, TEXTURE_TABLE_VALUES)?,
        })
    }
}

fn pointer(base: GameAddr, offset: usize) -> Option<GameAddr> {
    try_read::<usize>(base.checked_add(offset)?).map(GameAddr::new)
}

fn non_null(address: GameAddr) -> Option<GameAddr> {
    (address.get() != 0).then_some(address)
}

/// The address of element `index` of an array of `size`-byte elements at `array`.
fn element(array: GameAddr, index: usize, size: usize) -> Option<GameAddr> {
    array.checked_add(index.checked_mul(size)?)
}

#[cfg(test)]
mod tests;
