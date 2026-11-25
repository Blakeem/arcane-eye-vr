//! Byte offsets and sizes of the game struct fields the mod reads and writes, for game build
//! 4.1.1.7631656.
//!
//! Each matches bg3-vr's layout. The view and `PerCamera` offsets were measured with a debugger
//! breakpoint. The camera-cut field was checked by a static disassembly of the installed exe. The
//! pipeline, frame, level, second-view and shadow stage offsets come from bg3-vr's recipe and a
//! static disassembly. The game client offsets come from a static disassembly and debugger reads of
//! the state id. The `PerView` corner rays come from bg3-vr's layout and a dump of both blocks of
//! a turned eye view.

/// The camera controller pointer of a render view's `ReadState`, which the engine builds the view's
/// `PerView` and `PerCamera` blocks from.
pub const VIEW_READ_STATE_CONTROLLER: usize = 0x21D8;

/// The controller pointer a render view saw last. Begin-recording sets the view's camera-cut flag when
/// it differs from the `ReadState` controller pointer, then stores that pointer here.
pub const VIEW_LAST_SEEN_CONTROLLER: usize = 0x2B00;

/// `PerCamera` block 0 of a render view.
pub const VIEW_PER_CAMERA: usize = 0x2660;

/// `PerCamera` block 1 of a render view, with the layout of block 0.
pub const VIEW_PER_CAMERA_1: usize = 0x2740;

/// `PerView` block 0 of a render view, which belongs to `PerCamera` block 0.
pub const VIEW_PER_VIEW: usize = 0x2340;

/// `PerView` block 1 of a render view, which belongs to `PerCamera` block 1.
pub const VIEW_PER_VIEW_1: usize = 0x24D0;

/// The three corner rays in a `PerView` block, each three `f32` and one unused `f32`.
pub const PER_VIEW_CORNER_RAYS: [usize; 3] = [0x70, 0x80, 0x90];

/// The world-to-camera view matrix in a `PerCamera` block, 16 `f32` in column-major order.
pub const PER_CAMERA_VIEW: usize = 0x00;

/// The projection matrix in a `PerCamera` block, 16 `f32` in column-major order.
pub const PER_CAMERA_PROJECTION: usize = 0x40;

/// The near plane distance in a `PerCamera` block, one `f32`.
pub const PER_CAMERA_NEAR: usize = 0xC0;

/// The far plane distance in a `PerCamera` block, one `f32`.
pub const PER_CAMERA_FAR: usize = 0xC4;

/// The camera's world position in a `PerCamera` block, three `f32`.
pub const PER_CAMERA_POSITION: usize = 0xD0;

/// The size of the gameplay camera controller object.
pub const CONTROLLER_SIZE: usize = 0x330;

/// The world-to-camera view matrix in a camera controller, 16 `f32` in column-major order.
pub const CONTROLLER_VIEW: usize = 0x90;

/// The inverse of the view matrix in a camera controller, 16 `f32` in column-major order.
pub const CONTROLLER_INVERSE_VIEW: usize = 0xD0;

/// The camera's projection in a camera controller, 16 `f32` in column-major order, which the
/// view's `PerCamera` block follows. Measured with a probe build of the launch mode `vr`.
pub const CONTROLLER_PROJECTION: usize = 0x110;

/// The inverse of the projection in a camera controller, 16 `f32` in column-major order, which
/// `ScreenToWorldRay` reads (disassembly at RVA 0x4085EDC).
pub const CONTROLLER_INVERSE_PROJECTION: usize = 0x150;

/// The number of camera controller vtable slots a copy of the vtable holds, as bg3-vr's proxy copies.
pub const CONTROLLER_VTABLE_SLOTS: usize = 96;

/// The camera controller's vtable slot 9, `GetWorldTranslate`.
pub const CONTROLLER_WORLD_TRANSLATE_SLOT: usize = 9 * size_of::<usize>();

/// The main render view pointer in the render pipeline.
pub const PIPELINE_MAIN_VIEW: usize = 0xA0;

/// The special view pointer in the render pipeline. The special view composites the other views'
/// output and the UI to the back buffer.
pub const PIPELINE_SPECIAL_VIEW: usize = 0xA8;

/// The pipeline's vtable slot 88, the game pipeline setup.
pub const PIPELINE_SETUP_SLOT: usize = 88 * size_of::<usize>();

/// The player manager's vtable slot 1, `CreatePlayerView`.
pub const PLAYER_MANAGER_CREATE_VIEW_SLOT: usize = size_of::<usize>();

/// The stage resource manager pointer of a `RenderFrame`.
pub const FRAME_STAGE_RESOURCES: usize = 0x4250;

/// The view list of a `RenderFrame`, a pointer to an array of render view pointers.
pub const FRAME_VIEW_LIST: usize = 0x42E0;

/// The number of views in a `RenderFrame`'s view list, one `u32`.
pub const FRAME_VIEW_COUNT: usize = 0x42EC;

/// The per-view records of a stage resource manager, a pointer to an array of record pointers. A
/// record's first word is its render view.
pub const STAGE_RESOURCES_RECORDS: usize = 0x58;

/// The number of records of a stage resource manager, one `i32`.
pub const STAGE_RESOURCES_RECORD_COUNT: usize = 0x64;

/// The owner view of a render view's stage group. A view's own stages name the view itself.
pub const VIEW_STAGE_OWNER: usize = 0x20;

/// The texture table pointer of a render view. Every view of a pipeline holds the same table (a dump
/// of the main view, the special view and the eye view in the game).
pub const VIEW_TEXTURE_TABLE: usize = 0x18;

/// The bucket array of a texture table: one `i32` entry index per bucket, negative for an empty
/// bucket. This and the table offsets below come from a disassembly of the game's lookup and a dump
/// of the table.
pub const TEXTURE_TABLE_BUCKETS: usize = 0x78;

/// The bucket count of a texture table, an `i32`.
pub const TEXTURE_TABLE_BUCKET_COUNT: usize = 0x80;

/// The next array of a texture table: one `i32` entry index per entry, negative at the end of a
/// bucket's chain.
pub const TEXTURE_TABLE_NEXT: usize = 0x88;

/// The entry count of a texture table, an `i32`.
pub const TEXTURE_TABLE_ENTRY_COUNT: usize = 0x94;

/// The key array of a texture table: one `u32` key per entry.
pub const TEXTURE_TABLE_KEYS: usize = 0x98;

/// The value array of a texture table: one node pointer per entry.
pub const TEXTURE_TABLE_VALUES: usize = 0xA8;

/// The texture record pointer of a texture table node, null while the node has no texture.
pub const TEXTURE_NODE_RECORD: usize = 0x38;

/// The Direct3D texture pointer of a texture record.
pub const TEXTURE_RECORD_TEXTURE: usize = 0x00;

/// The stage array pointer of a render view's stage group.
pub const VIEW_STAGE_ARRAY: usize = 0x28;

/// The stage count of a render view's stage array, one `u32` (a dump of the special view and the
/// main view in the game).
pub const VIEW_STAGE_COUNT: usize = 0x34;

/// The UI enable byte of a composite stage. The stage's job draws the UI texture into the back
/// buffer only while it is nonzero (a write of 0 in the game, and the store in the stage's
/// constructor).
pub const COMPOSITE_STAGE_UI_ENABLED: usize = 0x2188;

/// The job context pointer of a render view.
pub const VIEW_JOB_CONTEXT: usize = 0x40;

/// The occlusion object pointer of a render view.
pub const VIEW_OCCLUSION: usize = 0x48;

/// The enabled byte of an occlusion object.
pub const OCCLUSION_ENABLED: usize = 0x80;

/// The Enabled byte of a render view's `WriteState`.
pub const VIEW_WRITE_STATE_ENABLED: usize = 0x21E0;

/// The camera controller pointer of a render view's `WriteState`.
pub const VIEW_WRITE_STATE_CONTROLLER: usize = 0x2260;

/// The `ReadState` of a render view, the render thread's copy of the `WriteState`. The frame sync
/// fills it (a dump of both states in the game).
pub const VIEW_READ_STATE: usize = 0x2158;

/// The `WriteState` of a render view, which the game thread fills. Its first byte is
/// [`VIEW_WRITE_STATE_ENABLED`].
pub const VIEW_WRITE_STATE: usize = 0x21E0;

/// The Scale of a view state, two `f32`: the view's share of the frame's width and of its height.
pub const STATE_SCALE: usize = 0x08;

/// The first of the three pixel viewports of a view state. The first two hold the render size and
/// the third the back buffer size (a dump at 2560x1600 and 3840x2160).
pub const STATE_PIXEL_VIEWPORTS: usize = 0x18;

/// The end of a view state's placement: its Scale, its Offset and its pixel viewports.
pub const STATE_PLACEMENT_END: usize = 0x78;

/// The size of a pixel viewport: a scale and an offset of two `f32` each, then its width, its
/// height, its x and its y in pixels as four `i32`.
pub const PIXEL_VIEWPORT_SIZE: usize = 0x20;

/// The pixel width of a pixel viewport, one `i32`.
pub const PIXEL_VIEWPORT_WIDTH: usize = 0x10;

/// The split-screen index byte of a render view. 0 is the primary player and 1 the second.
pub const VIEW_SPLIT_INDEX: usize = 0x2268;

/// The split-screen side of a render view, one `i32`. 0 is the left half and 1 the right half.
pub const VIEW_SPLIT_SIDE: usize = 0x226C;

/// The target atmosphere record pointer of a render view. The atmosphere setter copies its source
/// record into this record.
pub const VIEW_ATMOSPHERE_TARGET: usize = 0x2880;

/// The atmosphere request id of a render view, one `i32`, which the atmosphere setter stores. The
/// renderer's cull stage picks the view's environment probe by it.
pub const VIEW_ATMOSPHERE_REQUEST: usize = 0x2888;

/// The active atmosphere record pointer of a render view.
pub const VIEW_ATMOSPHERE: usize = 0x28A0;

/// The target lighting record pointer of a render view, in the role of [`VIEW_ATMOSPHERE_TARGET`].
pub const VIEW_LIGHTING_TARGET: usize = 0x28B8;

/// The lighting request id of a render view, one `i32`, which the lighting setter stores.
pub const VIEW_LIGHTING_REQUEST: usize = 0x28C0;

/// The active lighting record pointer of a render view.
pub const VIEW_LIGHTING: usize = 0x28D8;

/// The id in an atmosphere or lighting record, one `i32`.
pub const ENVIRONMENT_RECORD_ID: usize = 0x08;

/// The fog variant byte of a render view. The view's fog layer comes from this variant of its
/// atmosphere record. The game set it to 1 on each player view at the start of a cutscene and to 0
/// at its end (measured in one cutscene with a debugger).
pub const VIEW_FOG_VARIANT: usize = 0x2961;

/// The lighting lock byte of a render view. A set lock makes the lighting setter only record the
/// request.
pub const VIEW_LIGHTING_LOCK: usize = 0x2AEB;

/// The atmosphere lock byte of a render view, in the role of [`VIEW_LIGHTING_LOCK`].
pub const VIEW_ATMOSPHERE_LOCK: usize = 0x2AEC;

/// The scene pointer of a render view.
pub const VIEW_SCENE: usize = 0x2B10;

/// The player fields of a render view, [`VIEW_PLAYER_ROOM_SIZE`] bytes: the player's handle, the id
/// of the player's room at +0x10 and the player's region flags at +0x14. The game writes them into a
/// player's view in each frame's recording, and its outline draws show only objects of that room
/// (bg3-vr's reading of the code, a debugger watch of the writes and a copy that made the eye view
/// draw outlines).
pub const VIEW_PLAYER_ROOM: usize = 0x2BE8;

/// The size of a render view's player fields.
pub const VIEW_PLAYER_ROOM_SIZE: usize = 0x18;

/// The active byte of a scene. The engine's stage gate runs a view's stages only while it is set.
pub const SCENE_ACTIVE: usize = 0x08;

/// The render view's vtable slot 10, `SetCamera`.
pub const VIEW_SET_CAMERA_SLOT: usize = 10 * size_of::<usize>();

/// The current level pointer of the level manager.
pub const LEVEL_MANAGER_LEVEL: usize = 0x90;

/// The environment object pointer of a level.
pub const LEVEL_ENVIRONMENT: usize = 0x18;

/// The scene pointer of a level.
pub const LEVEL_SCENE: usize = 0x28;

/// The pointer to the environment ids in a level's environment object.
pub const LEVEL_ENVIRONMENT_IDS: usize = 0x08;

/// The level atmosphere id in a level's environment ids.
pub const LEVEL_ATMOSPHERE_ID: usize = 0x28;

/// The level lighting id in a level's environment ids.
pub const LEVEL_LIGHTING_ID: usize = 0x24;

/// The list in the id pool that the id release takes.
pub const ID_POOL_RELEASE_LIST: usize = 0xC600;

/// The state machine pointer of the game client.
pub const GAME_CLIENT_STATE_MACHINE: usize = 0x90;

/// The state id of the game client's state machine, one `u32`. The game thread writes it.
pub const STATE_MACHINE_STATE_ID: usize = 0x08;

/// The render view pointer of a shadow stage, the stage with stage id 0x02, which is entry 0x12 of
/// the view's stage array ([`VIEW_STAGE_ARRAY`]).
pub const SHADOW_STAGE_VIEW: usize = 0x08;

/// The render view pointer of an object data stage, the stage with stage id 0x47.
pub const OBJECT_STAGE_VIEW: usize = 0x08;

/// The slot index of an object data stage, an `i32`. The stage's execute stores its view's
/// split-screen index here each frame. The stage's job writes each visible object's offset in the
/// stage's ring buffer into the object's slot of that index, and the draws of the stage's view read
/// that slot (bg3-vr's reading of the code, and a disassembly of the execute).
pub const OBJECT_STAGE_SLOT: usize = 0x219C;

/// The parity byte of a shadow stage's distant shadow cache. A cache refresh switches it.
pub const SHADOW_CACHE_PARITY: usize = 0x21D8;

/// The records of a shadow stage's distant shadow cache, [`SHADOW_CACHE_RECORDS_SIZE`] bytes in world
/// space.
pub const SHADOW_CACHE_RECORDS: usize = 0x21E0;

/// The size of a shadow stage's distant shadow cache records.
pub const SHADOW_CACHE_RECORDS_SIZE: usize = 0x230;

/// The on byte of a shadow stage's distant shadow cache.
pub const SHADOW_CACHE_ON: usize = 0x241B;

const _: () = assert!(CONTROLLER_INVERSE_VIEW + 16 * size_of::<f32>() <= CONTROLLER_PROJECTION);
const _: () =
    assert!(CONTROLLER_PROJECTION + 16 * size_of::<f32>() <= CONTROLLER_INVERSE_PROJECTION);
const _: () = assert!(CONTROLLER_INVERSE_PROJECTION + 16 * size_of::<f32>() <= CONTROLLER_SIZE);
const _: () =
    assert!(CONTROLLER_WORLD_TRANSLATE_SLOT < CONTROLLER_VTABLE_SLOTS * size_of::<usize>());
