//! Stand-in game calls that record their arguments, for the tests of the create and the game update.

use game_bindings::frame_width::{FrameWidth, ViewState};
use game_bindings::offsets::{VIEW_FOG_VARIANT, VIEW_WRITE_STATE_CONTROLLER};
use game_bindings::pipeline::{Pipeline, RenderFrame};
use game_bindings::render_view::RenderView;
use game_mem::{GameAddr, Plain, WriteDest, try_write};

use super::engine::{Apply, CreateCalls, GameMemory, SetUpCalls};
use crate::raw::test_support::test_dest;

/// The width of the entire frame that the stand-in viewport setter stores: the render size twice and
/// the back buffer size.
pub(crate) const FRAME_WIDTH: FrameWidth = FrameWidth { scale: 1.0, pixels: [2560, 2560, 3840] };

/// A game call with its arguments, as the stand-in recorded it. The `flag` of each of the four
/// environment calls is the flag byte it passes to the game.
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
        flag: u8,
    },
    AttachLevelLighting {
        view: GameAddr,
        id: GameAddr,
        request: i32,
        flag: u8,
    },
    ReleaseId {
        list: GameAddr,
        id: i32,
    },
    SetAtmosphere {
        view: GameAddr,
        record: GameAddr,
        request: i32,
        flag: u8,
    },
    SetLighting {
        view: GameAddr,
        record: GameAddr,
        request: i32,
        flag: u8,
    },
    SetFogVariant {
        view: GameAddr,
        variant: u8,
    },
    SetViewport {
        view: GameAddr,
        scale: [f32; 2],
        offset: [f32; 2],
    },
}

/// Stand-in game calls. Each records its call and makes no other change, except that the level
/// atmosphere attach writes `level_request` into its request id, as the game may, and that
/// `SetCamera` and the fog variant setter store their value in the view while `stores` is set, as
/// the game does, and that the viewport setter panics while `viewport_panics` is set.
pub(crate) struct Recorder {
    pub(crate) calls: Vec<Call>,
    /// What the main frame getter returns.
    pub(crate) frame: GameAddr,
    /// What `CreatePlayerView` returns.
    pub(crate) created: GameAddr,
    pub(crate) level_request: i32,
    /// Whether `SetCamera` and the fog variant setter store their value in the view.
    pub(crate) stores: bool,
    /// Whether the viewport setter panics after it records its call, as a panic in the feature's
    /// code between two calls would.
    pub(crate) viewport_panics: bool,
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

/// Stores `value` at `offset` in the test view `view`.
fn store<T: Plain>(view: RenderView, offset: usize, value: T) {
    let address = view.address().checked_add(offset).expect("a test address");
    try_write(test_dest(address, size_of::<T>()), value).expect("a write to a test view");
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
        if self.stores {
            store(view, VIEW_WRITE_STATE_CONTROLLER, controller.get());
        }
    }

    fn set_scene(&mut self, view: RenderView, scene: GameAddr) {
        self.record(Call::SetScene { view: view.address(), scene });
    }

    fn attach_level_atmosphere(
        &mut self,
        view: RenderView,
        id: GameAddr,
        request: &mut i32,
        apply: Apply,
    ) {
        let (view, flag) = (view.address(), apply.flag());
        self.record(Call::AttachLevelAtmosphere { view, id, request: *request, flag });
        *request = self.level_request;
    }

    fn attach_level_lighting(
        &mut self,
        view: RenderView,
        id: GameAddr,
        request: &mut i32,
        apply: Apply,
    ) {
        let (view, flag) = (view.address(), apply.flag());
        self.record(Call::AttachLevelLighting { view, id, request: *request, flag });
    }

    fn release_id(&mut self, list: GameAddr, id: &mut i32) {
        self.record(Call::ReleaseId { list, id: *id });
    }

    fn set_atmosphere(
        &mut self,
        view: RenderView,
        record: GameAddr,
        request: &mut i32,
        apply: Apply,
    ) {
        let (view, flag) = (view.address(), apply.flag());
        self.record(Call::SetAtmosphere { view, record, request: *request, flag });
    }

    fn set_lighting(
        &mut self,
        view: RenderView,
        record: GameAddr,
        request: &mut i32,
        apply: Apply,
    ) {
        let (view, flag) = (view.address(), apply.flag());
        self.record(Call::SetLighting { view, record, request: *request, flag });
    }

    fn set_fog_variant(&mut self, view: RenderView, variant: u8) {
        self.record(Call::SetFogVariant { view: view.address(), variant });
        if self.stores {
            store(view, VIEW_FOG_VARIANT, variant);
        }
    }

    fn set_viewport(&mut self, view: RenderView, scale: [f32; 2], offset: [f32; 2]) {
        self.record(Call::SetViewport { view: view.address(), scale, offset });
        assert!(!self.viewport_panics, "the test's viewport setter panicked");
        let [scale_x, _] = scale;
        let entire = FrameWidth { scale: scale_x, ..FRAME_WIDTH };
        let stored = if entire.is_full() {
            entire
        } else {
            FrameWidth { scale: scale_x, ..FRAME_WIDTH.halved() }
        };
        stored
            .write(test_dest(view.address(), RenderView::WRITE_LEN), ViewState::Write)
            .expect("a write to a test view");
    }
}
