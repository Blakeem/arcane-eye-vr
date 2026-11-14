//! A pipeline, a frame, views, a level and a controller built in the test process's memory, and stand-in
//! game calls that record their arguments.
//!
//! The objects hold the title screen state the research measured: a main view with a controller, an
//! active scene, an atmosphere and a lighting, a special view and a level. The eye view is the view
//! the stand-in `CreatePlayerView` returns. It is already in the frame's view list and holds its
//! stage record, as the game's add-view leaves it.

use game_bindings::offsets::{
    ENVIRONMENT_RECORD_ID, FRAME_STAGE_RESOURCES, FRAME_VIEW_COUNT, FRAME_VIEW_LIST,
    LEVEL_ENVIRONMENT, LEVEL_ENVIRONMENT_IDS, LEVEL_MANAGER_LEVEL, LEVEL_SCENE, OCCLUSION_ENABLED,
    PIPELINE_MAIN_VIEW, PIPELINE_SETUP_SLOT, PIPELINE_SPECIAL_VIEW,
    PLAYER_MANAGER_CREATE_VIEW_SLOT, SCENE_ACTIVE, STAGE_RESOURCES_RECORD_COUNT,
    STAGE_RESOURCES_RECORDS, VIEW_ATMOSPHERE, VIEW_ATMOSPHERE_REQUEST, VIEW_JOB_CONTEXT,
    VIEW_LIGHTING, VIEW_LIGHTING_REQUEST, VIEW_OCCLUSION, VIEW_SCENE, VIEW_STAGE_ARRAY,
    VIEW_STAGE_OWNER, VIEW_WRITE_STATE_CONTROLLER,
};
use game_bindings::pipeline::{Pipeline, RenderFrame};
use game_bindings::render_view::RenderView;
use game_mem::{GameAddr, Plain, WriteDest, try_read, try_write};

use super::engine::{Addresses, CreateCalls, GameMemory, Held, SetUpCalls};
use crate::raw::test_support::test_dest;

/// `CreatePlayerView`'s address in the test world. No code runs there.
pub(crate) const CREATE_PLAYER_VIEW: GameAddr = GameAddr::new(0x7FF6_0000_1000);

/// The game pipeline setup's address in the test world. No code runs there.
pub(crate) const PIPELINE_SETUP: GameAddr = GameAddr::new(0x7FF6_0000_2000);

/// The main view's atmosphere id.
pub(crate) const ATMOSPHERE_ID: i32 = 11;

/// The main view's lighting id.
pub(crate) const LIGHTING_ID: i32 = 22;

/// The main view's atmosphere and lighting request ids.
pub(crate) const ATMOSPHERE_REQUEST: i32 = 33;
pub(crate) const LIGHTING_REQUEST: i32 = 44;

/// The bytes of a test view, which hold every field the feature reads and writes.
const VIEW_LEN: usize = VIEW_SCENE + 0x10;

/// The bytes of a test vtable, enough for pipeline slot 88. Its words are all 0 but the ones a test
/// sets.
const VTABLE_LEN: usize = size_of::<[usize; 96]>();

/// The bytes of a test view list or record list.
const LIST_LEN: usize = size_of::<[usize; 8]>();

/// Zeroed bytes in the test process's memory that the feature reaches only through guarded copies.
pub(crate) struct Block(Box<[u8]>);

impl Block {
    fn new(len: usize) -> Self {
        Self(vec![0_u8; len].into_boxed_slice())
    }

    pub(crate) fn address(&self) -> GameAddr {
        GameAddr::new(self.0.as_ptr().expose_provenance())
    }

    fn at(&self, offset: usize) -> GameAddr {
        self.address().checked_add(offset).expect("a test address")
    }

    pub(crate) fn set<T: Plain>(&self, offset: usize, value: T) {
        let dest = test_dest(self.at(offset), size_of::<T>());
        try_write(dest, value).expect("a write to test memory");
    }

    pub(crate) fn get<T: Plain>(&self, offset: usize) -> T {
        try_read(self.at(offset)).expect("a read of test memory")
    }
}

/// The globals, objects and vtables of the test world.
pub(crate) struct World {
    pub(crate) pipeline_global: Block,
    pub(crate) manager_global: Block,
    pub(crate) level_global: Block,
    pub(crate) id_pool_global: Block,
    pub(crate) view_vtable: Block,
    pub(crate) pipeline_vtable: Block,
    pub(crate) manager_vtable: Block,
    pub(crate) pipeline: Block,
    pub(crate) manager: Block,
    pub(crate) main: Block,
    pub(crate) special: Block,
    pub(crate) eye: Block,
    pub(crate) frame: Block,
    pub(crate) view_list: Block,
    pub(crate) resources: Block,
    pub(crate) record_list: Block,
    pub(crate) records: [Block; 3],
    pub(crate) level_manager: Block,
    pub(crate) level: Block,
    pub(crate) environment: Block,
    pub(crate) environment_ids: Block,
    pub(crate) id_pool: Block,
    pub(crate) scene: Block,
    pub(crate) atmosphere: Block,
    pub(crate) lighting: Block,
    pub(crate) main_occlusion: Block,
    pub(crate) eye_occlusion: Block,
    pub(crate) controller: Block,
    /// Stage arrays and job contexts, one per view.
    pub(crate) view_objects: [Block; 6],
}

impl World {
    /// The title screen world, with the eye view already made and listed.
    pub(crate) fn new() -> Self {
        let world = Self {
            pipeline_global: Block::new(8),
            manager_global: Block::new(8),
            level_global: Block::new(8),
            id_pool_global: Block::new(8),
            view_vtable: Block::new(VTABLE_LEN),
            pipeline_vtable: Block::new(VTABLE_LEN),
            manager_vtable: Block::new(VTABLE_LEN),
            pipeline: Block::new(0x100),
            manager: Block::new(0x10),
            main: Block::new(VIEW_LEN),
            special: Block::new(VIEW_LEN),
            eye: Block::new(VIEW_LEN),
            frame: Block::new(FRAME_VIEW_COUNT + 0x10),
            view_list: Block::new(LIST_LEN),
            resources: Block::new(0x80),
            record_list: Block::new(LIST_LEN),
            records: [Block::new(0x20), Block::new(0x20), Block::new(0x20)],
            level_manager: Block::new(0x100),
            level: Block::new(0x40),
            environment: Block::new(0x10),
            environment_ids: Block::new(0x40),
            id_pool: Block::new(0xC610),
            scene: Block::new(0x10),
            atmosphere: Block::new(0x10),
            lighting: Block::new(0x10),
            main_occlusion: Block::new(0x90),
            eye_occlusion: Block::new(0x90),
            controller: Block::new(0x10),
            view_objects: [0, 1, 2, 3, 4, 5].map(|_| Block::new(0x10)),
        };
        world.link_globals_and_vtables();
        world.link_views();
        world.link_frame(&[world.main.address(), world.eye.address(), world.special.address()]);
        world.link_level();
        world
    }

    fn link_globals_and_vtables(&self) {
        self.pipeline_global.set(0, self.pipeline.address().get());
        self.manager_global.set(0, self.manager.address().get());
        self.level_global.set(0, self.level_manager.address().get());
        self.id_pool_global.set(0, self.id_pool.address().get());
        self.pipeline.set(0, self.pipeline_vtable.address().get());
        self.pipeline_vtable.set(PIPELINE_SETUP_SLOT, PIPELINE_SETUP.get());
        self.manager.set(0, self.manager_vtable.address().get());
        self.manager_vtable.set(PLAYER_MANAGER_CREATE_VIEW_SLOT, CREATE_PLAYER_VIEW.get());
        self.pipeline.set(PIPELINE_MAIN_VIEW, self.main.address().get());
        self.pipeline.set(PIPELINE_SPECIAL_VIEW, self.special.address().get());
    }

    fn link_views(&self) {
        let views = [&self.main, &self.special, &self.eye];
        let (pairs, _) = self.view_objects.as_chunks::<2>();
        for (view, [stages, jobs]) in views.into_iter().zip(pairs) {
            view.set(0, self.view_vtable.address().get());
            view.set(VIEW_STAGE_OWNER, view.address().get());
            view.set(VIEW_STAGE_ARRAY, stages.address().get());
            view.set(VIEW_JOB_CONTEXT, jobs.address().get());
        }
        self.scene.set(SCENE_ACTIVE, 1_u8);
        self.atmosphere.set(ENVIRONMENT_RECORD_ID, ATMOSPHERE_ID);
        self.lighting.set(ENVIRONMENT_RECORD_ID, LIGHTING_ID);
        self.main.set(VIEW_WRITE_STATE_CONTROLLER, self.controller.address().get());
        self.main.set(VIEW_SCENE, self.scene.address().get());
        self.main.set(VIEW_ATMOSPHERE, self.atmosphere.address().get());
        self.main.set(VIEW_LIGHTING, self.lighting.address().get());
        self.main.set(VIEW_ATMOSPHERE_REQUEST, ATMOSPHERE_REQUEST);
        self.main.set(VIEW_LIGHTING_REQUEST, LIGHTING_REQUEST);
        self.main.set(VIEW_OCCLUSION, self.main_occlusion.address().get());
        self.eye.set(VIEW_OCCLUSION, self.eye_occlusion.address().get());
    }

    /// Sets the frame's view list and stage records to `views`, in that order.
    pub(crate) fn link_frame(&self, views: &[GameAddr]) {
        self.frame.set(FRAME_VIEW_LIST, self.view_list.address().get());
        self.frame.set(FRAME_STAGE_RESOURCES, self.resources.address().get());
        self.resources.set(STAGE_RESOURCES_RECORDS, self.record_list.address().get());
        for (index, (view, record)) in views.iter().zip(&self.records).enumerate() {
            let offset = index.checked_mul(size_of::<usize>()).expect("a test offset");
            self.view_list.set(offset, view.get());
            record.set(0, view.get());
            self.record_list.set(offset, record.address().get());
        }
        let count = u32::try_from(views.len()).expect("a test count");
        self.frame.set(FRAME_VIEW_COUNT, count);
        let records = i32::try_from(views.len()).expect("a test count");
        self.resources.set(STAGE_RESOURCES_RECORD_COUNT, records);
    }

    fn link_level(&self) {
        self.level_manager.set(LEVEL_MANAGER_LEVEL, self.level.address().get());
        self.level.set(LEVEL_SCENE, self.scene.address().get());
        self.level.set(LEVEL_ENVIRONMENT, self.environment.address().get());
        self.environment.set(LEVEL_ENVIRONMENT_IDS, self.environment_ids.address().get());
    }

    /// Gives the eye view the main view's controller, scene, atmosphere and lighting, as the set up
    /// leaves it in the game.
    pub(crate) fn make_eye_ready(&self) {
        self.eye.set(VIEW_WRITE_STATE_CONTROLLER, self.controller.address().get());
        self.eye.set(VIEW_SCENE, self.scene.address().get());
        self.eye.set(VIEW_ATMOSPHERE, self.atmosphere.address().get());
        self.eye.set(VIEW_LIGHTING, self.lighting.address().get());
    }

    pub(crate) fn addresses(&self) -> Addresses {
        Addresses {
            pipeline: self.pipeline_global.address(),
            player_manager: self.manager_global.address(),
            level_manager: self.level_global.address(),
            id_pool: self.id_pool_global.address(),
            game_render_view: self.view_vtable.address(),
            create_player_view: CREATE_PLAYER_VIEW,
            pipeline_setup: PIPELINE_SETUP,
        }
    }

    /// The eye view the create returns in this world.
    pub(crate) fn held(&self) -> Held {
        Held {
            eye: RenderView::new(self.eye.address()),
            pipeline: Pipeline::new(self.pipeline.address()),
            main: RenderView::new(self.main.address()),
            frame: RenderFrame::new(self.frame.address()),
        }
    }

    /// Stand-in game calls whose main frame is this world's frame and whose `CreatePlayerView`
    /// returns this world's eye view.
    pub(crate) fn recorder(&self) -> Recorder {
        Recorder {
            calls: Vec::with_capacity(64),
            frame: self.frame.address(),
            created: self.eye.address(),
            level_request: -1,
        }
    }

    /// The occlusion enabled byte of the main view and of the eye view.
    pub(crate) fn occlusion_bytes(&self) -> (u8, u8) {
        (self.main_occlusion.get(OCCLUSION_ENABLED), self.eye_occlusion.get(OCCLUSION_ENABLED))
    }
}

/// A game call with its arguments, as the stand-in recorded it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Call {
    MainFrame {
        pipeline: GameAddr,
    },
    CreatePlayerView {
        manager: GameAddr,
        frame: GameAddr,
    },
    /// The pipeline setup, with the view's split-screen index at the call.
    PipelineSetup {
        pipeline: GameAddr,
        view: GameAddr,
        split_index: u8,
    },
    SetCamera {
        view: GameAddr,
        controller: GameAddr,
    },
    SetScene {
        view: GameAddr,
        scene: GameAddr,
    },
    AttachLevelAtmosphere {
        view: GameAddr,
        id: GameAddr,
        request: i32,
    },
    AttachLevelLighting {
        view: GameAddr,
        id: GameAddr,
        request: i32,
    },
    ReleaseId {
        list: GameAddr,
        id: i32,
    },
    SetAtmosphere {
        view: GameAddr,
        record: GameAddr,
        request: i32,
    },
    SetLighting {
        view: GameAddr,
        record: GameAddr,
        request: i32,
    },
    SetViewport {
        view: GameAddr,
        scale: [f32; 2],
        offset: [f32; 2],
    },
}

/// Stand-in game calls. Each records its call and makes no other change, except that the level
/// atmosphere attach writes `level_request` into its request id, as the game may.
pub(crate) struct Recorder {
    pub(crate) calls: Vec<Call>,
    /// What the main frame getter returns.
    pub(crate) frame: GameAddr,
    /// What `CreatePlayerView` returns.
    pub(crate) created: GameAddr,
    pub(crate) level_request: i32,
}

impl Recorder {
    fn record(&mut self, call: Call) {
        self.calls.push(call);
    }

    /// The recorded calls, which the next call starts afresh.
    pub(crate) fn take(&mut self) -> Vec<Call> {
        let calls = self.calls.clone();
        self.calls.clear();
        calls
    }
}

impl GameMemory for Recorder {
    fn writes(&self, address: GameAddr, len: usize) -> WriteDest<'_> {
        test_dest(address, len)
    }
}

impl CreateCalls for Recorder {
    fn main_frame(&mut self, pipeline: Pipeline) -> GameAddr {
        self.record(Call::MainFrame { pipeline: pipeline.address() });
        self.frame
    }

    fn create_player_view(&mut self, manager: GameAddr, frame: RenderFrame) -> GameAddr {
        self.record(Call::CreatePlayerView { manager, frame: frame.address() });
        self.created
    }

    fn pipeline_setup(&mut self, pipeline: Pipeline, view: RenderView) {
        let split_index = view.content().map_or(u8::MAX, |content| content.split_index);
        let (pipeline, view) = (pipeline.address(), view.address());
        self.record(Call::PipelineSetup { pipeline, view, split_index });
    }
}

impl SetUpCalls for Recorder {
    fn main_frame(&mut self, pipeline: Pipeline) -> GameAddr {
        CreateCalls::main_frame(self, pipeline)
    }

    fn set_camera(&mut self, view: RenderView, controller: GameAddr) {
        self.record(Call::SetCamera { view: view.address(), controller });
    }

    fn set_scene(&mut self, view: RenderView, scene: GameAddr) {
        self.record(Call::SetScene { view: view.address(), scene });
    }

    fn attach_level_atmosphere(&mut self, view: RenderView, id: GameAddr, request: &mut i32) {
        self.record(Call::AttachLevelAtmosphere { view: view.address(), id, request: *request });
        *request = self.level_request;
    }

    fn attach_level_lighting(&mut self, view: RenderView, id: GameAddr, request: &mut i32) {
        self.record(Call::AttachLevelLighting { view: view.address(), id, request: *request });
    }

    fn release_id(&mut self, list: GameAddr, id: &mut i32) {
        self.record(Call::ReleaseId { list, id: *id });
    }

    fn set_atmosphere(&mut self, view: RenderView, record: GameAddr, request: &mut i32) {
        self.record(Call::SetAtmosphere { view: view.address(), record, request: *request });
    }

    fn set_lighting(&mut self, view: RenderView, record: GameAddr, request: &mut i32) {
        self.record(Call::SetLighting { view: view.address(), record, request: *request });
    }

    fn set_viewport(&mut self, view: RenderView, scale: [f32; 2], offset: [f32; 2]) {
        self.record(Call::SetViewport { view: view.address(), scale, offset });
    }
}
