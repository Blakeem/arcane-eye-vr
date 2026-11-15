//! The game functions and game memory the eye view feature reaches, as interfaces that tests replace,
//! and the addresses the feature reads.
//!
//! The core's implementations in `raw/eye_view.rs` call the game. Each takes the token of the thread
//! its calls run on. The feature checks every input with guarded reads before a call, since no call
//! into the game is fault-guarded.

use game_bindings::pipeline::{Pipeline, RenderFrame};
use game_bindings::render_view::RenderView;
use game_mem::{GameAddr, WriteDest};

/// The addresses the feature reads at a call site: the globals, the game render view's vtable and
/// the two functions the feature checks vtable slots against.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Addresses {
    /// The global that holds the render pipeline.
    pub(crate) pipeline: GameAddr,
    /// The global that holds the player manager.
    pub(crate) player_manager: GameAddr,
    /// The global that holds the level manager.
    pub(crate) level_manager: GameAddr,
    /// The global that holds the id pool.
    pub(crate) id_pool: GameAddr,
    /// The game render view's vtable, the first word of every view `CreatePlayerView` builds.
    pub(crate) game_render_view: GameAddr,
    /// `CreatePlayerView`, which the player manager's vtable slot 1 must hold.
    pub(crate) create_player_view: GameAddr,
    /// The game pipeline setup, which the pipeline's vtable slot 88 must hold.
    pub(crate) pipeline_setup: GameAddr,
}

/// The eye view the feature holds, with the pipeline, main view and main frame it was created for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Held {
    pub(crate) eye: RenderView,
    pub(crate) pipeline: Pipeline,
    pub(crate) main: RenderView,
    pub(crate) frame: RenderFrame,
}

/// When an environment call applies its source to the view, the call's flag byte.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Apply {
    /// Flag byte 0, which the game's scene setup passes. While a blend runs on the view, the setter
    /// keeps the source pointer and applies it at a later tick.
    AfterBlend,
    /// Flag byte 1. The setter applies the source at once.
    AtOnce,
}

impl Apply {
    /// The flag byte the environment functions take.
    pub(crate) const fn flag(self) -> u8 {
        match self {
            Self::AfterBlend => 0,
            Self::AtOnce => 1,
        }
    }
}

/// Writes to game memory.
pub(crate) trait GameMemory {
    /// The destination of guarded writes to the `len` bytes at `address`, an address read from game
    /// memory.
    fn writes(&self, address: GameAddr, len: usize) -> WriteDest<'_>;
}

/// The game calls of recipe steps 2, 3 and 6, on the render thread at the Present call site.
pub(crate) trait CreateCalls: GameMemory {
    /// The pipeline's main `RenderFrame`, or address 0 when it has none (step 2).
    fn main_frame(&mut self, pipeline: Pipeline) -> GameAddr;

    /// A new game render view in `frame`'s view list, or address 0 (step 3).
    fn create_player_view(&mut self, manager: GameAddr, frame: RenderFrame) -> GameAddr;

    /// Builds the stages of `view` (step 6).
    fn pipeline_setup(&mut self, pipeline: Pipeline, view: RenderView);
}

/// The game calls of recipe steps 8 to 15 and of the stale check, on the game thread at the game
/// update.
pub(crate) trait SetUpCalls: GameMemory {
    /// The pipeline's main `RenderFrame`, or address 0 when it has none.
    fn main_frame(&mut self, pipeline: Pipeline) -> GameAddr;

    /// `SetCamera` through the view's vtable slot 10 (step 8).
    fn set_camera(&mut self, view: RenderView, controller: GameAddr);

    /// Gives `view` the scene `scene` (step 9).
    fn set_scene(&mut self, view: RenderView, scene: GameAddr);

    /// Gives `view` the level atmosphere whose id is at `id` (step 10). The game may write an id to
    /// release into `request`. `apply` sets when the game applies the source.
    fn attach_level_atmosphere(
        &mut self,
        view: RenderView,
        id: GameAddr,
        request: &mut i32,
        apply: Apply,
    );

    /// Gives `view` the level lighting whose id is at `id` (step 11). `apply` sets when the game
    /// applies the source.
    fn attach_level_lighting(
        &mut self,
        view: RenderView,
        id: GameAddr,
        request: &mut i32,
        apply: Apply,
    );

    /// Releases `id` to the id pool list `list`.
    fn release_id(&mut self, list: GameAddr, id: &mut i32);

    /// Gives `view` the atmosphere record `record` with the request id `request` (step 12). `apply`
    /// sets when the game applies the source.
    fn set_atmosphere(
        &mut self,
        view: RenderView,
        record: GameAddr,
        request: &mut i32,
        apply: Apply,
    );

    /// Gives `view` the lighting record `record` with the request id `request` (step 13). `apply`
    /// sets when the game applies the source.
    fn set_lighting(&mut self, view: RenderView, record: GameAddr, request: &mut i32, apply: Apply);

    /// Gives `view` the fog variant `variant`, which rebuilds the view's fog.
    fn set_fog_variant(&mut self, view: RenderView, variant: u8);

    /// Places `view` at `offset` with the size `scale`, each a fraction of the frame (steps 14, 15).
    fn set_viewport(&mut self, view: RenderView, scale: [f32; 2], offset: [f32; 2]);
}
