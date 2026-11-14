//! Pipeline, frame, level manager and vtable reads against objects built in the test process's
//! memory.

use std::hint::black_box;

use game_mem::GameAddr;

use super::{LIST_LIMIT, Level, LevelManager, Pipeline, PipelineViews, RenderFrame, vtable_slot};
use crate::offsets::{
    FRAME_STAGE_RESOURCES, FRAME_VIEW_COUNT, FRAME_VIEW_LIST, LEVEL_ENVIRONMENT,
    LEVEL_ENVIRONMENT_IDS, LEVEL_MANAGER_LEVEL, LEVEL_SCENE, PIPELINE_MAIN_VIEW,
    PIPELINE_SPECIAL_VIEW, STAGE_RESOURCES_RECORD_COUNT, STAGE_RESOURCES_RECORDS,
};

/// Windows reserves the lowest 64 KiB of every process, so no read there succeeds.
const UNMAPPED: GameAddr = GameAddr::new(0x10);

fn address_of<T>(values: &[T]) -> GameAddr {
    GameAddr::new(values.as_ptr().expose_provenance())
}

fn put(bytes: &mut [u8], offset: usize, value: &[u8]) {
    bytes[offset..][..value.len()].copy_from_slice(value);
}

fn word(value: usize) -> [u8; 8] {
    value.to_ne_bytes()
}

#[test]
fn a_pipeline_yields_its_main_and_special_views() {
    let mut pipeline = vec![0_u8; 0x100];
    put(&mut pipeline, PIPELINE_MAIN_VIEW, &word(0xA000));
    put(&mut pipeline, PIPELINE_SPECIAL_VIEW, &word(0xB000));
    let pipeline = black_box(pipeline);
    let views = Pipeline::new(address_of(&pipeline)).views();
    let expected = PipelineViews { main: GameAddr::new(0xA000), special: GameAddr::new(0xB000) };
    assert_eq!(views, Some(expected));
    assert_eq!(Pipeline::new(UNMAPPED).views(), None);
}

/// A frame whose view list holds `views` and whose stage resource manager holds one record per
/// entry of `records`, each record's first word that entry.
struct TestFrame {
    frame: Vec<u8>,
    _views: Vec<usize>,
    _manager: Vec<u8>,
    _record_list: Vec<usize>,
    _records: Vec<[usize; 4]>,
}

impl TestFrame {
    fn new(views: &[usize], records: &[usize], record_count: i32) -> Self {
        let views = black_box(views.to_vec());
        let records: Vec<[usize; 4]> =
            black_box(records.iter().map(|view| [*view, 0, 0, 0]).collect());
        let record_list: Vec<usize> =
            black_box(records.iter().map(|record| record.as_ptr().expose_provenance()).collect());
        let mut manager = vec![0_u8; 0x80];
        put(&mut manager, STAGE_RESOURCES_RECORDS, &word(address_of(&record_list).get()));
        put(&mut manager, STAGE_RESOURCES_RECORD_COUNT, &record_count.to_ne_bytes());
        let manager = black_box(manager);
        let mut frame = vec![0_u8; FRAME_VIEW_COUNT + 0x10];
        put(&mut frame, FRAME_STAGE_RESOURCES, &word(address_of(&manager).get()));
        put(&mut frame, FRAME_VIEW_LIST, &word(address_of(&views).get()));
        let count = u32::try_from(views.len()).expect("a test count");
        put(&mut frame, FRAME_VIEW_COUNT, &count.to_ne_bytes());
        let frame = black_box(frame);
        Self {
            frame,
            _views: views,
            _manager: manager,
            _record_list: record_list,
            _records: records,
        }
    }

    fn frame(&self) -> RenderFrame {
        RenderFrame::new(address_of(&self.frame))
    }
}

#[test]
fn a_frame_lists_its_views_and_records_and_nothing_else() {
    let test = TestFrame::new(&[0xA000, 0xE000, 0xB000], &[0xA000, 0xE000], 2);
    let frame = test.frame();
    assert_eq!(frame.view_count(), Some(3));
    assert_eq!(frame.lists_view(GameAddr::new(0xE000)), Some(true));
    assert_eq!(frame.lists_view(GameAddr::new(0xF000)), Some(false));
    assert_eq!(frame.has_record(GameAddr::new(0xE000)), Some(true));
    assert_eq!(frame.has_record(GameAddr::new(0xB000)), Some(false));
}

#[test]
fn an_unreadable_or_oversized_list_reads_as_none() {
    assert_eq!(RenderFrame::new(UNMAPPED).lists_view(GameAddr::new(0xA000)), None);
    assert_eq!(RenderFrame::new(UNMAPPED).has_record(GameAddr::new(0xA000)), None);
    let negative = TestFrame::new(&[0xA000], &[0xA000], -1);
    assert_eq!(negative.frame().has_record(GameAddr::new(0xA000)), None);
    let views: Vec<usize> = (1..=usize::try_from(LIST_LIMIT).expect("a limit") + 1).collect();
    let oversized = TestFrame::new(&views, &[], 0);
    assert_eq!(oversized.frame().lists_view(GameAddr::new(1)), None);
}

#[test]
fn a_level_manager_yields_the_level_scene_and_environment_ids() {
    let ids = black_box(vec![0_u8; 0x40]);
    let mut environment = vec![0_u8; 0x10];
    put(&mut environment, LEVEL_ENVIRONMENT_IDS, &word(address_of(&ids).get()));
    let environment = black_box(environment);
    let mut level = vec![0_u8; 0x40];
    put(&mut level, LEVEL_ENVIRONMENT, &word(address_of(&environment).get()));
    put(&mut level, LEVEL_SCENE, &word(0x5CE0));
    let level = black_box(level);
    let mut manager = vec![0_u8; 0x100];
    put(&mut manager, LEVEL_MANAGER_LEVEL, &word(address_of(&level).get()));
    let manager = black_box(manager);
    let read = LevelManager::new(address_of(&manager)).level();
    let expected = Level { scene: GameAddr::new(0x5CE0), environment_ids: address_of(&ids) };
    assert_eq!(read, Some(expected));
}

#[test]
fn a_level_manager_without_a_level_yields_zero_and_an_unmapped_one_nothing() {
    let manager = black_box(vec![0_u8; 0x100]);
    let read = LevelManager::new(address_of(&manager)).level();
    let empty = Level { scene: GameAddr::new(0), environment_ids: GameAddr::new(0) };
    assert_eq!(read, Some(empty));
    assert_eq!(LevelManager::new(UNMAPPED).level(), None);
}

#[test]
fn a_vtable_slot_yields_its_function_and_an_unmapped_object_nothing() {
    let vtable = black_box(vec![0x100_usize, 0x200, 0x300]);
    let object = black_box(vec![address_of(&vtable).get()]);
    let slot = vtable_slot(address_of(&object), 2 * size_of::<usize>());
    assert_eq!(slot, Some(GameAddr::new(0x300)));
    assert_eq!(vtable_slot(UNMAPPED, 0), None);
}
