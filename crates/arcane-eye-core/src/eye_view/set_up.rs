//! The game update's reads of the held eye view, recipe steps 8 to 13, which give the eye view the
//! main view's camera controller, the level's scene and the main view's atmosphere and lighting, and
//! the fog variant step and the player room step, which give it the main view's fog variant and
//! player fields.
//!
//! All run on the game thread at the game update, since the steps change game thread state. The
//! feature re-reads the pipeline, the main view and the main frame every frame, since the game may
//! rebuild them or clear the eye view's camera and scene at a level unload. The game's logic gives
//! only a player's view a new camera controller and fog variant, at each cut of a cutscene, so the
//! feature compares both every frame. The game writes the player fields of a player's view in each
//! frame's recording, which ends before the next game update, so the feature compares them every
//! frame too.

use std::time::Instant;

use diag::warn_every;
use game_bindings::offsets::{ID_POOL_RELEASE_LIST, LEVEL_ATMOSPHERE_ID, LEVEL_LIGHTING_ID};
use game_bindings::pipeline::{Level, LevelManager, Pipeline};
use game_bindings::render_view::{Environment, PlayerRoom, RenderView, ViewContent};
use game_mem::{GameAddr, WriteError, try_read};

use tracing::info;

use super::decide::{Observed, Readiness, Stale, readiness};
use super::engine::{Addresses, Apply, Held, SetUpCalls};
use super::place::Placement;
use super::{LOG_PERIOD, Shared};

/// The request id that names no request, which the game's own scene setup passes to the level
/// attaches.
const NO_REQUEST: i32 = -1;

/// A read of the game update that failed, after which the feature makes no call this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub(crate) enum ReadFault {
    #[error("the pipeline global could not be read")]
    PipelineGlobal,
    #[error("the pipeline's views could not be read")]
    PipelineViews,
    #[error("the main frame's view list could not be read")]
    ViewList,
    #[error("the eye view's vtable could not be read")]
    EyeVtable,
    #[error("the main or eye view's content could not be read")]
    Content,
}

/// A guard of steps 8 to 13 that failed, after which the feature makes no call of the set up.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub(crate) enum SetUpGuard {
    #[error("the eye view's split-screen index is not the primary player's")]
    SplitIndex,
    #[error("the level manager or the id pool global could not be read")]
    GlobalsUnreadable,
    #[error("the level's scene is not the main view's scene")]
    LevelSceneDiffers,
    #[error("the level has no environment ids")]
    NoLevelEnvironment,
    #[error("the id pool is null")]
    NoIdPool,
    #[error("the eye view's atmosphere lock is set")]
    AtmosphereLocked,
    #[error("the eye view's lighting lock is set")]
    LightingLocked,
    #[error("the main view's atmosphere target is null")]
    NoMainAtmosphereTarget,
    #[error("the main view's lighting target is null")]
    NoMainLightingTarget,
}

/// The content of the main view and of the eye view in one frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Contents {
    pub(crate) main: ViewContent,
    pub(crate) eye: ViewContent,
}

/// What the game update read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Update {
    pub(crate) observed: Observed,
    /// The pipeline's main view this frame, or `None` when it holds none or was not read.
    pub(crate) main: Option<RenderView>,
    /// Both views' content, for a live eye view.
    pub(crate) contents: Option<Contents>,
}

impl Update {
    const fn of(observed: Observed, main: Option<RenderView>) -> Self {
        Self { observed, main, contents: None }
    }
}

/// Reads the game state at the game update whose pipeline is `update_pipeline`, for the eye view
/// `held`.
///
/// # Errors
///
/// Returns the read that failed. The main frame getter runs only for the held pipeline, after its
/// views read.
pub(crate) fn observe(
    calls: &mut impl SetUpCalls,
    addresses: &Addresses,
    held: Option<Held>,
    update_pipeline: GameAddr,
) -> Result<Update, ReadFault> {
    let pipeline = try_read::<usize>(addresses.pipeline).ok_or(ReadFault::PipelineGlobal)?;
    let pipeline = Pipeline::new(GameAddr::new(pipeline));
    if pipeline.address() != update_pipeline {
        return Ok(Update::of(Observed::UpdateOfOtherPipeline, None));
    }
    let Some(held) = held else { return Ok(Update::of(Observed::UpdateWithoutEye, None)) };
    let views = pipeline.views().ok_or(ReadFault::PipelineViews)?;
    let main = (views.main.get() != 0).then_some(RenderView::new(views.main));
    if pipeline != held.pipeline {
        return Ok(Update::of(Observed::Stale(Stale::PipelineChanged), main));
    }
    if main != Some(held.main) {
        return Ok(Update::of(Observed::Stale(Stale::MainViewChanged), main));
    }
    if calls.main_frame(pipeline) != held.frame.address() {
        return Ok(Update::of(Observed::Stale(Stale::FrameChanged), main));
    }
    match held.frame.lists_view(held.eye.address()) {
        Some(true) => {}
        Some(false) => return Ok(Update::of(Observed::Stale(Stale::NotListed), main)),
        None => return Err(ReadFault::ViewList),
    }
    let vtable = held.eye.vtable().ok_or(ReadFault::EyeVtable)?;
    if vtable != addresses.game_render_view {
        return Ok(Update::of(Observed::Stale(Stale::NotGameRenderView), main));
    }
    let contents = contents(held).ok_or(ReadFault::Content)?;
    let observed = Observed::Live(readiness(&contents.main, &contents.eye));
    Ok(Update { observed, main, contents: Some(contents) })
}

fn contents(held: Held) -> Option<Contents> {
    Some(Contents { main: held.main.content()?, eye: held.eye.content()? })
}

/// Whether the eye view of `held` is ready now, read again after a set up run changed it.
pub(crate) fn is_ready(held: Held) -> bool {
    contents(held).is_some_and(|after| readiness(&after.main, &after.eye) == Readiness::Ready)
}

/// One environment step pair: the level attach (step 10 or 11) and the main view's target record
/// (step 12 or 13).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct EnvironmentSteps {
    /// The level's environment id that the attach takes.
    level_id: GameAddr,
    /// The main view's target record that the setter takes. The main view's active record holds
    /// default fog layers, and only its target holds the level's.
    target: GameAddr,
    /// The main view's request id that the setter stores. The renderer picks a view's environment
    /// probe by its atmosphere request id, so -1 would leave the eye view with no probe.
    request: i32,
}

/// The calls of one set up run, each `None` when the eye view's value already equals the main
/// view's.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct SetUpPlan {
    camera: Option<GameAddr>,
    scene: Option<GameAddr>,
    atmosphere: Option<EnvironmentSteps>,
    /// The id pool list that step 10's attach may hand an id to release.
    release_list: Option<GameAddr>,
    lighting: Option<EnvironmentSteps>,
    fog_variant: Option<u8>,
    player_room: Option<PlayerRoom>,
}

impl SetUpPlan {
    /// Which of the camera, scene, atmosphere, lighting, fog variant and player room steps the plan
    /// runs, for the log.
    pub(crate) const fn steps(&self) -> [bool; 6] {
        [
            self.camera.is_some(),
            self.scene.is_some(),
            self.atmosphere.is_some(),
            self.lighting.is_some(),
            self.fog_variant.is_some(),
            self.player_room.is_some(),
        ]
    }
}

/// The calls that give the eye view the main view's values in `contents`, after every guard.
///
/// # Errors
///
/// Returns the first guard that failed.
pub(crate) fn plan(addresses: &Addresses, contents: &Contents) -> Result<SetUpPlan, SetUpGuard> {
    let Contents { main, eye } = contents;
    if eye.split_index != 0 {
        return Err(SetUpGuard::SplitIndex);
    }
    let mut plan = SetUpPlan::default();
    if eye.controller != main.controller {
        plan.camera = Some(main.controller);
    }
    if eye.fog_variant != main.fog_variant {
        plan.fog_variant = Some(main.fog_variant);
    }
    if eye.player_room != main.player_room {
        plan.player_room = Some(main.player_room);
    }
    let needs_scene = eye.scene != main.scene;
    let needs_atmosphere = eye.atmosphere.active_id != main.atmosphere.active_id;
    let needs_lighting = eye.lighting.active_id != main.lighting.active_id;
    if !(needs_scene || needs_atmosphere || needs_lighting) {
        return Ok(plan);
    }
    let level = read_level(addresses)?;
    if needs_scene {
        if level.scene != main.scene {
            return Err(SetUpGuard::LevelSceneDiffers);
        }
        plan.scene = Some(level.scene);
    }
    if needs_atmosphere {
        if eye.atmosphere_lock != 0 {
            return Err(SetUpGuard::AtmosphereLocked);
        }
        if main.atmosphere.target_id.is_none() {
            return Err(SetUpGuard::NoMainAtmosphereTarget);
        }
        plan.atmosphere = Some(environment_steps(&level, LEVEL_ATMOSPHERE_ID, &main.atmosphere)?);
        plan.release_list = Some(release_list(addresses)?);
    }
    if needs_lighting {
        if eye.lighting_lock != 0 {
            return Err(SetUpGuard::LightingLocked);
        }
        if main.lighting.target_id.is_none() {
            return Err(SetUpGuard::NoMainLightingTarget);
        }
        plan.lighting = Some(environment_steps(&level, LEVEL_LIGHTING_ID, &main.lighting)?);
    }
    Ok(plan)
}

/// Runs the calls of `plan` on `eye`, in recipe order, then the plan's write of the player fields.
///
/// # Errors
///
/// Returns the [`WriteError`] of a player field write that faulted, after every call ran.
pub(crate) fn run(
    calls: &mut impl SetUpCalls,
    eye: RenderView,
    plan: &SetUpPlan,
) -> Result<(), WriteError> {
    if let Some(controller) = plan.camera {
        calls.set_camera(eye, controller);
    }
    if let Some(scene) = plan.scene {
        calls.set_scene(eye, scene);
    }
    if let Some(steps) = plan.atmosphere {
        let mut level_request = NO_REQUEST;
        calls.attach_level_atmosphere(eye, steps.level_id, &mut level_request, Apply::AfterBlend);
        if level_request != NO_REQUEST
            && let Some(list) = plan.release_list
        {
            calls.release_id(list, &mut level_request);
        }
    }
    if let Some(steps) = plan.lighting {
        let mut level_request = NO_REQUEST;
        calls.attach_level_lighting(eye, steps.level_id, &mut level_request, Apply::AfterBlend);
    }
    // The level attaches start a blend, which would delay an `AfterBlend` source to a later tick.
    if let Some(steps) = plan.atmosphere {
        let mut request = steps.request;
        calls.set_atmosphere(eye, steps.target, &mut request, Apply::AtOnce);
    }
    if let Some(steps) = plan.lighting {
        let mut request = steps.request;
        calls.set_lighting(eye, steps.target, &mut request, Apply::AtOnce);
    }
    if let Some(variant) = plan.fog_variant {
        calls.set_fog_variant(eye, variant);
    }
    if let Some(room) = plan.player_room {
        let view = calls.writes(eye.address(), RenderView::WRITE_LEN);
        RenderView::write_player_room(view, room)?;
    }
    Ok(())
}

fn read_level(addresses: &Addresses) -> Result<Level, SetUpGuard> {
    let manager =
        try_read::<usize>(addresses.level_manager).ok_or(SetUpGuard::GlobalsUnreadable)?;
    LevelManager::new(GameAddr::new(manager)).level().ok_or(SetUpGuard::GlobalsUnreadable)
}

fn environment_steps(
    level: &Level,
    id_offset: usize,
    main: &Environment,
) -> Result<EnvironmentSteps, SetUpGuard> {
    if level.environment_ids.get() == 0 {
        return Err(SetUpGuard::NoLevelEnvironment);
    }
    let level_id =
        level.environment_ids.checked_add(id_offset).ok_or(SetUpGuard::NoLevelEnvironment)?;
    Ok(EnvironmentSteps { level_id, target: main.target, request: main.request })
}

fn release_list(addresses: &Addresses) -> Result<GameAddr, SetUpGuard> {
    let pool = try_read::<usize>(addresses.id_pool).ok_or(SetUpGuard::GlobalsUnreadable)?;
    if pool == 0 {
        return Err(SetUpGuard::NoIdPool);
    }
    GameAddr::new(pool).checked_add(ID_POOL_RELEASE_LIST).ok_or(SetUpGuard::NoIdPool)
}

/// Runs the differing steps of 8 to 13, the fog variant step and the player room step after their
/// guards, and returns the placement of this frame. A run that leaves the eye view ready places the
/// views side by side. A failed guard or any other run gives the main view the entire frame and
/// waits [`RETRY_PERIOD`](super::decide::RETRY_PERIOD) before the next run.
pub(super) fn attempt(
    addresses: &Addresses,
    calls: &mut impl SetUpCalls,
    shared: &mut Shared,
    update: &Update,
    now: Instant,
) -> Placement {
    let (Some(held), Some(contents)) = (shared.held, update.contents) else {
        return Placement::FullFrame;
    };
    let plan = match plan(addresses, &contents) {
        Ok(plan) => plan,
        Err(guard) => {
            shared.set_up_retry.wait(now);
            warn_every!(shared.set_up_log, LOG_PERIOD, guard = %guard, "eye view set up waits");
            return Placement::FullFrame;
        }
    };
    if let Err(error) = run(calls, held.eye, &plan) {
        warn_every!(shared.set_up_log, LOG_PERIOD, error = %error, "eye view set up write failed");
    }
    let ready = is_ready(held);
    let [camera, scene, atmosphere, lighting, fog_variant, player_room] = plan.steps();
    // A ready eye view runs no set up until the main view changes, and any other run waits
    // RETRY_PERIOD, so this line needs no rate-limit macro.
    info!(
        camera,
        scene, atmosphere, lighting, fog_variant, player_room, ready, "eye view set up ran"
    );
    if ready {
        return Placement::SideBySide;
    }
    shared.set_up_retry.wait(now);
    Placement::FullFrame
}
