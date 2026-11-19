//! A pipeline, a main view and a texture table in test memory, for tests of the game source.

use std::iter;

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

/// A pipeline whose main view holds a table of one bucket and one chain of entries, whose records
/// name textures.
pub(crate) struct World {
    pub(crate) global: Block,
    pub(crate) pipeline: Block,
    pub(crate) view: Block,
    /// The record of the first entry.
    pub(crate) record: Block,
    table: Block,
    arrays: [Block; 4],
    /// The node of each entry and the record of each entry after the first.
    nodes: Vec<Block>,
    records: Vec<Block>,
}

impl World {
    /// A world whose scene key names the object at `texture`.
    pub(crate) fn new(texture: usize) -> Self {
        Self::with_key(TextureKey::SCENE, texture)
    }

    /// A world whose `key` names the object at `texture`.
    pub(crate) fn with_key(key: TextureKey, texture: usize) -> Self {
        Self::with_keys(&[(key, texture)])
    }

    /// A world whose table holds one entry for each `(key, texture)` of `entries`, in one chain.
    pub(crate) fn with_keys(entries: &[(TextureKey, usize)]) -> Self {
        let count = entries.len();
        let bytes = |each: usize| each.checked_mul(count).expect("a test size").max(8);
        let world = Self {
            global: Block::new(8),
            pipeline: Block::new(0x100),
            view: Block::new(0x40),
            record: Block::new(0x10),
            table: Block::new(0xB0),
            arrays: [8, bytes(4), bytes(4), bytes(8)].map(Block::new),
            nodes: entries.iter().map(|_| Block::new(0x40)).collect(),
            records: entries.iter().skip(1).map(|_| Block::new(0x10)).collect(),
        };
        let [buckets, next, keys, values] = &world.arrays;
        world.global.set(0, world.pipeline.address().get());
        world.pipeline.set(PIPELINE_MAIN_VIEW, world.view.address().get());
        world.view.set(VIEW_TEXTURE_TABLE, world.table.address().get());
        world.table.set(TEXTURE_TABLE_BUCKET_COUNT, 1_i32);
        world.table.set(TEXTURE_TABLE_ENTRY_COUNT, i32::try_from(count).expect("a test count"));
        world.table.set(TEXTURE_TABLE_BUCKETS, buckets.address().get());
        world.table.set(TEXTURE_TABLE_NEXT, next.address().get());
        world.table.set(TEXTURE_TABLE_KEYS, keys.address().get());
        world.table.set(TEXTURE_TABLE_VALUES, values.address().get());
        let records = iter::once(&world.record).chain(&world.records);
        for (index, (((key, texture), node), record)) in
            entries.iter().zip(&world.nodes).zip(records).enumerate()
        {
            let at = |size: usize| index.checked_mul(size).expect("a test offset");
            let following = index.checked_add(1).filter(|following| *following < count);
            let following = following.map_or(-1, |entry| i32::try_from(entry).expect("an entry"));
            next.set(at(4), following);
            keys.set(at(4), key.0);
            values.set(at(8), node.address().get());
            node.set(TEXTURE_NODE_RECORD, record.address().get());
            record.set(TEXTURE_RECORD_TEXTURE, *texture);
        }
        world
    }

    /// The game source of `key` in this world.
    pub(crate) fn source(&self, key: TextureKey) -> GameSource {
        GameSource::new(key, self.global.address())
    }
}
