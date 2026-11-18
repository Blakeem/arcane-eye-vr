//! Composite stage lookups against a view and stages built in the test process's memory.

use std::hint::black_box;

use game_mem::GameAddr;

use super::CompositeStage;
use crate::render_view::RenderView;

/// Windows reserves the lowest 64 KiB of every process, so no read there succeeds.
const UNMAPPED: usize = 0x10;

/// The vtable address the test's composite stage holds. No read goes there.
const COMPOSITE_VTABLE: usize = 0x7FF6_58C7_00C0;

/// The vtable address of the test's other stages.
const OTHER_VTABLE: usize = 0x7FF6_58B4_0FC8;

/// The size of a test stage, past the UI enable byte at 0x2188.
const STAGE_SIZE: usize = 0x2190;

fn address_of<T>(values: &[T]) -> usize {
    values.as_ptr().expose_provenance()
}

fn put(bytes: &mut [u8], offset: usize, value: &[u8]) {
    bytes[offset..][..value.len()].copy_from_slice(value);
}

/// A stage whose first word is `vtable` and whose UI enable byte, at 0x2188, is `enabled`.
fn stage(vtable: usize, enabled: u8) -> Vec<u8> {
    let mut bytes = vec![0_u8; STAGE_SIZE];
    put(&mut bytes, 0, &vtable.to_ne_bytes());
    put(&mut bytes, 0x2188, &[enabled]);
    black_box(bytes)
}

/// A view whose stage array, at 0x28, holds two other stages and then the composite stage, as the
/// special view's did in the game. Its stage count is at 0x34.
struct World {
    view: Vec<u8>,
    array: Vec<usize>,
    _others: [Vec<u8>; 2],
    composite: Vec<u8>,
}

impl World {
    fn new() -> Self {
        let others = [stage(OTHER_VTABLE, 0), stage(OTHER_VTABLE, 0)];
        let composite = stage(COMPOSITE_VTABLE, 1);
        let array =
            black_box(vec![address_of(&others[0]), address_of(&others[1]), address_of(&composite)]);
        let mut view = vec![0_u8; 0x40];
        put(&mut view, 0x28, &address_of(&array).to_ne_bytes());
        put(&mut view, 0x34, &3_u32.to_ne_bytes());
        Self { view: black_box(view), array, _others: others, composite }
    }

    fn found(&self) -> Option<CompositeStage> {
        let view = RenderView::new(GameAddr::new(address_of(&self.view)));
        CompositeStage::of(view, GameAddr::new(COMPOSITE_VTABLE))
    }
}

#[test]
fn the_stage_with_the_composite_vtable_is_found_with_its_ui_enable_byte() {
    let world = World::new();
    let stage = world.found().expect("the composite stage");
    assert_eq!(stage.address(), GameAddr::new(address_of(&world.composite)));
    assert_eq!(stage.ui_enabled(), Some(1));
}

#[test]
fn a_null_entry_before_the_composite_stage_is_passed_over() {
    let mut world = World::new();
    world.array[0] = 0;
    assert!(world.found().is_some());
}

#[test]
fn an_array_without_the_composite_stage_yields_nothing() {
    let mut world = World::new();
    put(&mut world.view, 0x34, &2_u32.to_ne_bytes());
    assert_eq!(world.found(), None, "the count ends before the composite stage");
    put(&mut world.view, 0x34, &0_u32.to_ne_bytes());
    assert_eq!(world.found(), None, "an empty array");
}

#[test]
fn a_count_above_64_an_unmapped_stage_and_an_unmapped_view_yield_nothing() {
    let mut world = World::new();
    put(&mut world.view, 0x34, &65_u32.to_ne_bytes());
    assert_eq!(world.found(), None, "65 stages");
    put(&mut world.view, 0x34, &3_u32.to_ne_bytes());
    world.array[1] = UNMAPPED;
    assert_eq!(world.found(), None, "an unmapped stage before the composite stage");
    let unmapped = RenderView::new(GameAddr::new(UNMAPPED));
    assert_eq!(CompositeStage::of(unmapped, GameAddr::new(COMPOSITE_VTABLE)), None);
    assert_eq!(CompositeStage::new(GameAddr::new(UNMAPPED)).ui_enabled(), None);
}
