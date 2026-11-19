//! Render view and camera controller reads against objects built in the test process's memory.

use std::hint::black_box;

use game_mem::GameAddr;

use super::{
    CameraController, Environment, PlayerRoom, ReadState, RenderView, ViewContent, ViewParts,
};
use crate::camera::BlockPair;
use crate::offsets::{
    CONTROLLER_SIZE, CONTROLLER_VIEW, CONTROLLER_VTABLE_SLOTS, CONTROLLER_WORLD_TRANSLATE_SLOT,
    ENVIRONMENT_RECORD_ID, PER_CAMERA_FAR, PER_CAMERA_NEAR, PER_CAMERA_POSITION,
    PER_CAMERA_PROJECTION, PER_CAMERA_VIEW, SCENE_ACTIVE, VIEW_ATMOSPHERE, VIEW_ATMOSPHERE_LOCK,
    VIEW_ATMOSPHERE_REQUEST, VIEW_ATMOSPHERE_TARGET, VIEW_FOG_VARIANT, VIEW_JOB_CONTEXT,
    VIEW_LAST_SEEN_CONTROLLER, VIEW_LIGHTING_LOCK, VIEW_LIGHTING_REQUEST, VIEW_LIGHTING_TARGET,
    VIEW_OCCLUSION, VIEW_PER_CAMERA, VIEW_PLAYER_ROOM, VIEW_PLAYER_ROOM_SIZE,
    VIEW_READ_STATE_CONTROLLER, VIEW_SCENE, VIEW_SPLIT_INDEX, VIEW_STAGE_ARRAY, VIEW_STAGE_OWNER,
    VIEW_WRITE_STATE_CONTROLLER,
};

/// Windows reserves the lowest 64 KiB of every process, so no read there succeeds.
const UNMAPPED: GameAddr = GameAddr::new(0x10);

/// A controller address that a test view names. No read goes there.
const CONTROLLER: usize = 0x1234_5678;

fn address_of(bytes: &[u8]) -> GameAddr {
    GameAddr::new(bytes.as_ptr().expose_provenance())
}

fn put(bytes: &mut [u8], offset: usize, value: &[u8]) {
    bytes[offset..][..value.len()].copy_from_slice(value);
}

fn floats(values: &[f32]) -> Vec<u8> {
    values.iter().flat_map(|value| value.to_ne_bytes()).collect()
}

#[test]
fn a_view_yields_its_controller_and_its_per_camera_fields() {
    let mut view = vec![0_u8; VIEW_PER_CAMERA + 0x100];
    put(&mut view, VIEW_READ_STATE_CONTROLLER, &CONTROLLER.to_ne_bytes());
    let block = VIEW_PER_CAMERA;
    let matrix: Vec<f32> = (0_u8..16).map(f32::from).collect();
    let projection: Vec<f32> = (16_u8..32).map(f32::from).collect();
    put(&mut view, block + PER_CAMERA_VIEW, &floats(&matrix));
    put(&mut view, block + PER_CAMERA_PROJECTION, &floats(&projection));
    put(&mut view, block + PER_CAMERA_NEAR, &floats(&[0.5]));
    put(&mut view, block + PER_CAMERA_FAR, &floats(&[900.0]));
    put(&mut view, block + PER_CAMERA_POSITION, &floats(&[1.0, 2.0, 3.0]));
    let view = black_box(view);
    let render_view = RenderView::new(address_of(&view));
    let controller = render_view.read_state_controller();
    assert_eq!(controller, Some(CameraController(GameAddr::new(CONTROLLER))));
    let read = render_view.per_camera().expect("a readable block");
    assert_eq!(read.view.to_vec(), matrix);
    assert_eq!(read.projection.to_vec(), projection);
    assert_eq!((read.near, read.far), (0.5, 900.0));
    assert_eq!(read.position, [1.0, 2.0, 3.0]);
}

#[test]
fn each_block_pair_reads_its_own_per_camera_block() {
    let mut view = vec![0_u8; 0x2740 + 0x100];
    put(&mut view, 0x2660 + PER_CAMERA_FAR, &floats(&[900.0]));
    put(&mut view, 0x2740 + PER_CAMERA_FAR, &floats(&[250.0]));
    let view = black_box(view);
    let render_view = RenderView::new(address_of(&view));
    let far = |pair| render_view.per_camera_of(pair).map(|block| block.far);
    assert_eq!(far(BlockPair::First), Some(900.0));
    assert_eq!(far(BlockPair::Second), Some(250.0));
    assert_eq!(render_view.per_camera(), render_view.per_camera_of(BlockPair::First));
    assert_eq!(RenderView::new(UNMAPPED).per_camera_of(BlockPair::Second), None);
}

#[test]
fn a_null_controller_pointer_and_an_unmapped_view_yield_nothing() {
    let view = black_box(vec![0_u8; VIEW_PER_CAMERA + 0x100]);
    assert_eq!(RenderView::new(address_of(&view)).read_state_controller(), None);
    assert_eq!(RenderView::new(UNMAPPED).read_state_controller(), None);
    assert_eq!(RenderView::new(UNMAPPED).per_camera(), None);
}

#[test]
fn a_controller_yields_its_slot_9_function_and_an_unmapped_one_nothing() {
    let mut vtable = vec![0_usize; 12];
    vtable[CONTROLLER_WORLD_TRANSLATE_SLOT / size_of::<usize>()] = 0xABCD;
    let vtable = black_box(vtable);
    let object = black_box([vtable.as_ptr().expose_provenance()]);
    let controller = CameraController(GameAddr::new(object.as_ptr().expose_provenance()));
    assert_eq!(controller.world_translate_fn(), Some(GameAddr::new(0xABCD)));
    assert_eq!(CameraController(UNMAPPED).world_translate_fn(), None);
}

#[test]
fn read_state_tells_a_null_pointer_from_an_unreadable_one() {
    let mut view = vec![0_u8; VIEW_PER_CAMERA];
    let null = RenderView::new(address_of(&view)).read_state();
    put(&mut view, VIEW_READ_STATE_CONTROLLER, &CONTROLLER.to_ne_bytes());
    let view = black_box(view);
    let set = RenderView::new(address_of(&view)).read_state();
    assert_eq!(null, Some(ReadState::Null));
    let named = CameraController::new(GameAddr::new(CONTROLLER));
    assert_eq!(set, Some(ReadState::Controller(named)));
    assert_eq!(named.address(), GameAddr::new(CONTROLLER));
    assert_eq!(RenderView::new(UNMAPPED).read_state(), None);
}

#[test]
fn the_last_seen_controller_reads_a_null_pointer_and_nothing_from_an_unmapped_view() {
    let view = black_box(vec![0_u8; VIEW_LAST_SEEN_CONTROLLER + 8]);
    let last_seen = RenderView::new(address_of(&view)).last_seen_controller();
    assert_eq!(last_seen, Some(GameAddr::new(0)));
    assert_eq!(RenderView::new(UNMAPPED).last_seen_controller(), None);
}

#[test]
fn a_controller_copy_holds_its_bytes_and_view_matrix_and_the_vtable_its_slots() {
    let slots: Vec<usize> = (1..=CONTROLLER_VTABLE_SLOTS).collect();
    let slots = black_box(slots);
    let matrix: Vec<f32> = (0_u8..16).map(|value| f32::from(value) + 0.5).collect();
    let mut object = vec![0x5A_u8; CONTROLLER_SIZE];
    put(&mut object, 0, &slots.as_ptr().expose_provenance().to_ne_bytes());
    put(&mut object, CONTROLLER_VIEW, &floats(&matrix));
    let object = black_box(object);
    let controller = CameraController::new(address_of(&object));
    let copy = controller.copy().expect("a readable controller");
    assert_eq!(copy.bytes().as_slice(), object.as_slice());
    assert_eq!(copy.view().to_vec(), matrix);
    let read = controller.vtable_slots().map(|read| read.to_vec());
    assert_eq!(read.as_ref(), Some(&slots));
    assert_eq!(CameraController::new(UNMAPPED).copy(), None);
    assert_eq!(CameraController::new(UNMAPPED).vtable_slots(), None);
}

/// The size of a test view that holds every field the second view's recipe reads.
const VIEW_LEN: usize = RenderView::WRITE_LEN;

#[test]
fn a_view_yields_its_parts_and_an_unmapped_one_nothing() {
    let mut view = vec![0_u8; VIEW_LEN];
    put(&mut view, 0, &0x55F4_usize.to_ne_bytes());
    put(&mut view, VIEW_STAGE_OWNER, &0x2000_usize.to_ne_bytes());
    put(&mut view, VIEW_STAGE_ARRAY, &0x2800_usize.to_ne_bytes());
    put(&mut view, VIEW_JOB_CONTEXT, &0x4000_usize.to_ne_bytes());
    put(&mut view, VIEW_OCCLUSION, &0x4800_usize.to_ne_bytes());
    let view = black_box(view);
    let render_view = RenderView::new(address_of(&view));
    let expected = ViewParts {
        vtable: GameAddr::new(0x55F4),
        stage_owner: GameAddr::new(0x2000),
        stage_array: GameAddr::new(0x2800),
        job_context: GameAddr::new(0x4000),
    };
    assert_eq!(render_view.parts(), Some(expected));
    assert_eq!(render_view.occlusion(), Some(GameAddr::new(0x4800)));
    assert_eq!(RenderView::new(UNMAPPED).parts(), None);
    assert_eq!(RenderView::new(UNMAPPED).occlusion(), None);
}

#[test]
fn a_view_yields_its_content_with_the_scene_byte_and_the_environment_and_request_ids() {
    let mut scene = vec![0_u8; 0x10];
    scene[SCENE_ACTIVE] = 1;
    let scene = black_box(scene);
    let atmosphere = black_box(record(17));
    let atmosphere_target = black_box(record(18));
    let lighting_target = black_box(record(23));
    let mut view = vec![0_u8; VIEW_LEN];
    put(&mut view, VIEW_WRITE_STATE_CONTROLLER, &CONTROLLER.to_ne_bytes());
    put(&mut view, VIEW_SCENE, &address_of(&scene).get().to_ne_bytes());
    put(&mut view, VIEW_ATMOSPHERE, &address_of(&atmosphere).get().to_ne_bytes());
    put(&mut view, VIEW_ATMOSPHERE_TARGET, &address_of(&atmosphere_target).get().to_ne_bytes());
    put(&mut view, VIEW_ATMOSPHERE_REQUEST, &33_i32.to_ne_bytes());
    put(&mut view, VIEW_LIGHTING_TARGET, &address_of(&lighting_target).get().to_ne_bytes());
    put(&mut view, VIEW_LIGHTING_REQUEST, &(-1_i32).to_ne_bytes());
    view[VIEW_FOG_VARIANT] = 4;
    let mut room = [0_u8; VIEW_PLAYER_ROOM_SIZE];
    for (byte, value) in room.iter_mut().zip(0x40_u8..) {
        *byte = value;
    }
    put(&mut view, VIEW_PLAYER_ROOM, &room);
    view[VIEW_SPLIT_INDEX] = 1;
    view[VIEW_ATMOSPHERE_LOCK] = 2;
    view[VIEW_LIGHTING_LOCK] = 3;
    let view = black_box(view);
    let expected = ViewContent {
        controller: GameAddr::new(CONTROLLER),
        scene: address_of(&scene),
        scene_active: 1,
        atmosphere: Environment {
            active_id: Some(17),
            target: address_of(&atmosphere_target),
            target_id: Some(18),
            request: 33,
        },
        lighting: Environment {
            active_id: None,
            target: address_of(&lighting_target),
            target_id: Some(23),
            request: -1,
        },
        fog_variant: 4,
        player_room: PlayerRoom(room),
        split_index: 1,
        atmosphere_lock: 2,
        lighting_lock: 3,
    };
    assert_eq!(RenderView::new(address_of(&view)).content(), Some(expected));
    assert_eq!(RenderView::new(UNMAPPED).content(), None);
}

#[test]
fn a_null_scene_reads_as_an_inactive_scene() {
    let view = black_box(vec![0_u8; VIEW_LEN]);
    let content = RenderView::new(address_of(&view)).content().expect("a readable view");
    assert_eq!((content.scene, content.scene_active), (GameAddr::new(0), 0));
}

#[test]
fn a_null_target_record_has_no_id_and_an_unmapped_one_yields_no_content() {
    let null_targets = black_box(vec![0_u8; VIEW_LEN]);
    let content = RenderView::new(address_of(&null_targets)).content().expect("a readable view");
    assert_eq!((content.atmosphere.target_id, content.lighting.target_id), (None, None));
    for target_offset in [VIEW_ATMOSPHERE_TARGET, VIEW_LIGHTING_TARGET] {
        let mut view = vec![0_u8; VIEW_LEN];
        put(&mut view, target_offset, &UNMAPPED.get().to_ne_bytes());
        let view = black_box(view);
        assert_eq!(RenderView::new(address_of(&view)).content(), None, "{target_offset:#X}");
    }
}

/// An environment record whose id is `id`.
fn record(id: i32) -> Vec<u8> {
    let mut record = vec![0_u8; 0x10];
    put(&mut record, ENVIRONMENT_RECORD_ID, &id.to_ne_bytes());
    record
}
