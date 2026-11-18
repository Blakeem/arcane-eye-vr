//! A pipeline, a main view and a texture table in test memory, for tests of the game source.

use game_bindings::offsets::{
    PIPELINE_MAIN_VIEW, TEXTURE_NODE_RECORD, TEXTURE_RECORD_TEXTURE, TEXTURE_TABLE_BUCKET_COUNT,
    TEXTURE_TABLE_BUCKETS, TEXTURE_TABLE_ENTRY_COUNT, TEXTURE_TABLE_KEYS, TEXTURE_TABLE_NEXT,
    TEXTURE_TABLE_VALUES, VIEW_TEXTURE_TABLE,
};
use game_bindings::texture_table::TextureKey;
use game_mem::{GameAddr, Plain, try_read, try_write};

use super::GameSource;
use crate::raw::test_support::test_dest;

/// Zeroed bytes in the test process's memory.
pub(crate) struct Block(Box<[u8]>);

impl Block {
    pub(crate) fn new(len: usize) -> Self {
        Self(vec![0_u8; len].into_boxed_slice())
    }

    pub(crate) fn address(&self) -> GameAddr {
        GameAddr::new(self.0.as_ptr().expose_provenance())
    }

    pub(crate) fn set<T: Plain>(&self, offset: usize, value: T) {
        let field = self.address().checked_add(offset).expect("a test address");
        try_write(test_dest(field, size_of::<T>()), value).expect("a write to test memory");
    }

    pub(crate) fn get<T: Plain>(&self, offset: usize) -> T {
        let field = self.address().checked_add(offset).expect("a test address");
        try_read(field).expect("a read of test memory")
    }
}

/// A pipeline whose main view holds a table of one bucket and one entry, whose record names a
/// texture.
pub(crate) struct World {
    pub(crate) global: Block,
    pub(crate) pipeline: Block,
    pub(crate) view: Block,
    pub(crate) record: Block,
    table: Block,
    arrays: [Block; 5],
}

impl World {
    /// A world whose scene key names the object at `texture`.
    pub(crate) fn new(texture: usize) -> Self {
        Self::with_key(TextureKey::SCENE, texture)
    }

    /// A world whose `key` names the object at `texture`.
    pub(crate) fn with_key(key: TextureKey, texture: usize) -> Self {
        let world = Self {
            global: Block::new(8),
            pipeline: Block::new(0x100),
            view: Block::new(0x40),
            record: Block::new(0x10),
            table: Block::new(0xB0),
            arrays: [8, 8, 8, 8, 0x40].map(Block::new),
        };
        let [buckets, next, keys, values, node] = &world.arrays;
        world.global.set(0, world.pipeline.address().get());
        world.pipeline.set(PIPELINE_MAIN_VIEW, world.view.address().get());
        world.view.set(VIEW_TEXTURE_TABLE, world.table.address().get());
        world.table.set(TEXTURE_TABLE_BUCKET_COUNT, 1_i32);
        world.table.set(TEXTURE_TABLE_ENTRY_COUNT, 1_i32);
        world.table.set(TEXTURE_TABLE_BUCKETS, buckets.address().get());
        world.table.set(TEXTURE_TABLE_NEXT, next.address().get());
        world.table.set(TEXTURE_TABLE_KEYS, keys.address().get());
        world.table.set(TEXTURE_TABLE_VALUES, values.address().get());
        next.set(0, -1_i32);
        keys.set(0, key.0);
        values.set(0, node.address().get());
        node.set(TEXTURE_NODE_RECORD, world.record.address().get());
        world.record.set(TEXTURE_RECORD_TEXTURE, texture);
        world
    }

    /// The game source of `key` in this world.
    pub(crate) fn source(&self, key: TextureKey) -> GameSource {
        GameSource::new(key, self.global.address())
    }
}
