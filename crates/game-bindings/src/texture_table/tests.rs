//! Texture table lookups against a table built in the test process's memory.

use std::hint::black_box;

use game_mem::GameAddr;

use super::{TextureKey, TextureTable};
use crate::offsets::{
    TEXTURE_NODE_RECORD, TEXTURE_TABLE_BUCKET_COUNT, TEXTURE_TABLE_BUCKETS,
    TEXTURE_TABLE_ENTRY_COUNT, TEXTURE_TABLE_KEYS, TEXTURE_TABLE_NEXT, TEXTURE_TABLE_VALUES,
    VIEW_TEXTURE_TABLE,
};
use crate::render_view::RenderView;

/// Windows reserves the lowest 64 KiB of every process, so no read there succeeds.
const UNMAPPED: usize = 0x10;

/// A texture address that a test record names. No read goes there.
const TEXTURE: usize = 0x7FF6_1234_5000;

/// The bucket count of the test table.
const BUCKETS: i32 = 5;

fn address_of<T>(values: &[T]) -> usize {
    values.as_ptr().expose_provenance()
}

fn put(bytes: &mut [u8], offset: usize, value: &[u8]) {
    bytes[offset..][..value.len()].copy_from_slice(value);
}

/// A table of three entries whose keys 7, 12 and 3 fill the buckets 2, 2 and 3. The entries of
/// bucket 2 chain from entry 1 to entry 0.
struct World {
    table: Vec<u8>,
    buckets: Vec<i32>,
    next: Vec<i32>,
    _keys: Vec<u32>,
    values: Vec<usize>,
    /// The node of each entry. Only entry 0's node holds a record.
    _nodes: [Vec<u8>; 3],
    _record: Vec<usize>,
}

impl World {
    fn new() -> Self {
        let record = black_box(vec![TEXTURE, 0]);
        let mut with_record = vec![0_u8; 0x40];
        put(&mut with_record, TEXTURE_NODE_RECORD, &address_of(&record).to_ne_bytes());
        let nodes = black_box([with_record, vec![0_u8; 0x40], vec![0_u8; 0x40]]);
        let buckets = black_box(vec![-1, -1, 1, 2, -1]);
        let next = black_box(vec![-1, 0, -1]);
        let keys = black_box(vec![7_u32, 12, 3]);
        let values = black_box(nodes.each_ref().map(|node| address_of(node)).to_vec());
        let mut table = vec![0_u8; 0xB0];
        put(&mut table, TEXTURE_TABLE_BUCKET_COUNT, &BUCKETS.to_ne_bytes());
        put(&mut table, TEXTURE_TABLE_ENTRY_COUNT, &3_i32.to_ne_bytes());
        put(&mut table, TEXTURE_TABLE_BUCKETS, &address_of(&buckets).to_ne_bytes());
        put(&mut table, TEXTURE_TABLE_NEXT, &address_of(&next).to_ne_bytes());
        put(&mut table, TEXTURE_TABLE_KEYS, &address_of(&keys).to_ne_bytes());
        put(&mut table, TEXTURE_TABLE_VALUES, &address_of(&values).to_ne_bytes());
        let table = black_box(table);
        Self { table, buckets, next, _keys: keys, values, _nodes: nodes, _record: record }
    }

    fn table(&self) -> TextureTable {
        TextureTable::new(GameAddr::new(address_of(&self.table)))
    }

    fn texture(&self, key: u32) -> Option<GameAddr> {
        self.table().texture(TextureKey(key))
    }
}

#[test]
fn a_key_at_the_end_of_its_bucket_chain_yields_its_records_texture() {
    let world = World::new();
    assert_eq!(world.texture(7), Some(GameAddr::new(TEXTURE)));
}

#[test]
fn a_key_without_a_record_an_absent_key_and_an_empty_bucket_yield_nothing() {
    let world = World::new();
    assert_eq!(world.texture(12), None, "entry 1's node holds no record");
    assert_eq!(world.texture(3), None, "entry 2's node holds no record");
    assert_eq!(world.texture(17), None, "bucket 2 holds no key 17");
    assert_eq!(world.texture(4), None, "bucket 4 is empty");
}

#[test]
fn a_chain_that_loops_and_an_entry_index_past_the_entries_yield_nothing() {
    let mut world = World::new();
    world.next[0] = 1;
    assert_eq!(world.texture(17), None, "the chain 1, 0, 1 never ends");
    world.buckets[3] = 9;
    assert_eq!(world.texture(3), None, "entry 9 is past the 3 entries");
}

#[test]
fn a_table_without_buckets_a_null_node_and_an_unmapped_table_yield_nothing() {
    let mut world = World::new();
    world.values[0] = 0;
    assert_eq!(world.texture(7), None, "a null node");
    for count in [0_i32, -5] {
        put(&mut world.table, TEXTURE_TABLE_BUCKET_COUNT, &count.to_ne_bytes());
        assert_eq!(world.texture(7), None, "bucket count {count}");
    }
    assert_eq!(TextureTable::new(GameAddr::new(UNMAPPED)).texture(TextureKey::UI), None);
}

#[test]
fn a_view_yields_its_table_and_a_view_without_one_nothing() {
    let mut view = vec![0_u8; 0x40];
    put(&mut view, VIEW_TEXTURE_TABLE, &0x5000_usize.to_ne_bytes());
    let view = black_box(view);
    let table = TextureTable::of(RenderView::new(GameAddr::new(address_of(&view))));
    assert_eq!(table, Some(TextureTable::new(GameAddr::new(0x5000))));
    let empty = black_box(vec![0_u8; 0x40]);
    assert_eq!(TextureTable::of(RenderView::new(GameAddr::new(address_of(&empty)))), None);
    assert_eq!(TextureTable::of(RenderView::new(GameAddr::new(UNMAPPED))), None);
}
