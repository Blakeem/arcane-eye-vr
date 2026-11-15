//! The eye view feature's unsafe edge: the game update hook install, the game functions it calls and
//! the write destinations of the game objects it writes.
//!
//! Each function address comes from its signature's one match in the loaded game exe, and `SetCamera`
//! from slot 10 of the game render view's vtable. Only [`locate`] fills [`EngineFns`], so no other
//! address reaches a call. The feature checks every argument with guarded reads before a call, since
//! a call into the game is never fault-guarded.

use std::mem::{self, ManuallyDrop};
use std::ptr;

use game_bindings::offsets::VIEW_SET_CAMERA_SLOT;
use game_bindings::pipeline::{Pipeline, RenderFrame};
use game_bindings::render_view::RenderView;
use game_bindings::signatures::{
    G_ID_POOL, G_LEVEL_MANAGER, G_PIPELINE, G_PLAYER_MANAGER, GAME_PIPELINE_SETUP,
    GAME_UPDATE_LOGIC_END, ID_POOL_RELEASE, PIPELINE_MAIN_FRAME, Signature,
    VIEW_ATTACH_LEVEL_ATMOSPHERE, VIEW_ATTACH_LEVEL_LIGHTING, VIEW_CREATE_PLAYER,
    VIEW_SET_ATMOSPHERE, VIEW_SET_FOG_VARIANT, VIEW_SET_LIGHTING, VIEW_SET_SCENE,
    VIEW_SET_VIEWPORT, VT_GAME_RENDER_VIEW,
};
use game_mem::{GameAddr, WriteDest, try_read};
use hooks::{GameThread, HookError, MidHook, MidThunk, RenderThread};

use crate::eye_view::EyeViewError;
use crate::eye_view::engine::{Addresses, Apply, CreateCalls, GameMemory, SetUpCalls};
use crate::install::{GameCode, InstallError};

type MainFrameFn = extern "system" fn(pipeline: usize) -> usize;
type CreatePlayerViewFn = extern "system" fn(manager: usize, frame: usize) -> usize;
type PipelineSetupFn = extern "system" fn(pipeline: usize, view: usize);
type SetCameraFn = extern "system" fn(view: usize, controller: usize, flag: u8);
type SetSceneFn = extern "system" fn(view: usize, scene: usize);
type EnvironmentFn =
    extern "system" fn(view: usize, source: usize, request: *mut i32, blend: f32, flag: u8);
type ReleaseIdFn = extern "system" fn(list: usize, id: *mut i32);
type SetFogVariantFn = extern "system" fn(view: usize, variant: u8);
type SetViewportFn =
    extern "system" fn(view: usize, scale: *const [f32; 2], offset: *const [f32; 2]);

/// The blend time the environment calls take, as the game's set-player-view-scene passes it.
const NO_BLEND: f32 = 0.0;

/// The flag byte `SetCamera` takes, as the game's level unload passes it.
const NO_FLAG: u8 = 0;

/// The game functions the eye view feature calls, each at its address in the loaded game exe.
#[derive(Debug)]
pub(crate) struct EngineFns {
    main_frame: usize,
    create_player_view: usize,
    pipeline_setup: usize,
    set_camera: usize,
    set_scene: usize,
    attach_level_atmosphere: usize,
    attach_level_lighting: usize,
    release_id: usize,
    set_atmosphere: usize,
    set_lighting: usize,
    set_fog_variant: usize,
    set_viewport: usize,
}

/// What [`locate`] found in the game exe's code.
pub(crate) struct Located {
    pub(crate) fns: EngineFns,
    pub(crate) addresses: Addresses,
    /// The end of the game update's logic calls, the hook site.
    pub(crate) site: GameAddr,
}

/// Resolves every signature of the eye view feature in `code` and reads `SetCamera` from the game
/// render view's vtable.
///
/// # Errors
///
/// Returns the first signature that yields no address, or [`EyeViewError::SetCameraSlot`].
pub(crate) fn locate(code: &GameCode) -> Result<Located, EyeViewError> {
    let addresses = Addresses {
        pipeline: located(code, &G_PIPELINE)?,
        player_manager: located(code, &G_PLAYER_MANAGER)?,
        level_manager: located(code, &G_LEVEL_MANAGER)?,
        id_pool: located(code, &G_ID_POOL)?,
        game_render_view: located(code, &VT_GAME_RENDER_VIEW)?,
        create_player_view: located(code, &VIEW_CREATE_PLAYER)?,
        pipeline_setup: located(code, &GAME_PIPELINE_SETUP)?,
    };
    let site = located(code, &GAME_UPDATE_LOGIC_END)?;
    let fns = EngineFns {
        main_frame: located(code, &PIPELINE_MAIN_FRAME)?.get(),
        create_player_view: addresses.create_player_view.get(),
        pipeline_setup: addresses.pipeline_setup.get(),
        set_camera: set_camera_slot(addresses.game_render_view)?,
        set_scene: located(code, &VIEW_SET_SCENE)?.get(),
        attach_level_atmosphere: located(code, &VIEW_ATTACH_LEVEL_ATMOSPHERE)?.get(),
        attach_level_lighting: located(code, &VIEW_ATTACH_LEVEL_LIGHTING)?.get(),
        release_id: located(code, &ID_POOL_RELEASE)?.get(),
        set_atmosphere: located(code, &VIEW_SET_ATMOSPHERE)?.get(),
        set_lighting: located(code, &VIEW_SET_LIGHTING)?.get(),
        set_fog_variant: located(code, &VIEW_SET_FOG_VARIANT)?.get(),
        set_viewport: located(code, &VIEW_SET_VIEWPORT)?.get(),
    };
    Ok(Located { fns, addresses, site })
}

fn located(code: &GameCode, signature: &Signature) -> Result<GameAddr, InstallError> {
    Ok(code.locate(signature)?)
}

/// `SetCamera`, read from slot 10 of the game render view's vtable `vtable`.
fn set_camera_slot(vtable: GameAddr) -> Result<usize, EyeViewError> {
    let slot = vtable.checked_add(VIEW_SET_CAMERA_SLOT);
    let set_camera = slot.and_then(try_read::<usize>).filter(|function| *function != 0);
    set_camera.ok_or(EyeViewError::SetCameraSlot)
}

/// Installs `thunk` as the mid hook at `address`, the end of the game update's logic calls.
///
/// `address` must be the one match of the `game_update_logic_end` signature in the loaded game exe.
/// The hook never drops, so it stays installed for the process life, and no game thread runs a freed
/// stub.
pub(crate) fn install_game_update_hook(
    address: GameAddr,
    thunk: MidThunk,
) -> Result<ManuallyDrop<MidHook>, HookError> {
    // SAFETY: the signature's one match holds `mov rdx,rbx`, `mov rcx,[rdi+98h]` and a `call` with a
    // 32-bit relative target at the site, 15 bytes with no RIP-relative data operand and no short
    // jump. SafetyHook's 5-byte jump moves the first two and its 14-byte jump all three, and it
    // relocates a moved `call`. The `je` before the site targets the site's first byte, and no jump
    // targets a later byte of the 15. The game thread runs the game update, the only thread the game
    // thunk's token allows: its one caller is the game thread's run function, through pipeline
    // vtable slot 81.
    let hook = unsafe { hooks::install_mid_hook(address, thunk) }?;
    Ok(ManuallyDrop::new(hook))
}

/// The game calls of the create, on the render thread.
pub(crate) struct RenderCalls<'calls> {
    fns: &'calls EngineFns,
    _thread: &'calls RenderThread,
}

/// The game calls of the set up, the placement and the stale check, on the game thread.
pub(crate) struct GameCalls<'calls> {
    fns: &'calls EngineFns,
    _thread: &'calls GameThread,
}

impl<'calls> RenderCalls<'calls> {
    pub(crate) const fn new(fns: &'calls EngineFns, thread: &'calls RenderThread) -> Self {
        Self { fns, _thread: thread }
    }
}

impl<'calls> GameCalls<'calls> {
    pub(crate) const fn new(fns: &'calls EngineFns, thread: &'calls GameThread) -> Self {
        Self { fns, _thread: thread }
    }
}

/// The destination of guarded writes to the `len` bytes at `address`.
fn game_writes<'calls>(address: GameAddr, len: usize) -> WriteDest<'calls> {
    // SAFETY: the feature passes only addresses it read from game memory, the views and occlusion
    // objects the game holds. Those bytes are game memory or unmapped, and no Rust reference covers
    // them.
    unsafe { WriteDest::new(address, len) }
}

/// The function at `address` as a function pointer of type `F`.
///
/// # Safety
///
/// `address` is a function of the loaded game exe whose signature and ABI are `F`.
unsafe fn function<F: Copy>(address: usize) -> F {
    let pointer = ptr::with_exposed_provenance::<()>(address);
    // SAFETY: the caller guarantees that `address` is a function of type `F`, and `F` is a function
    // pointer type of pointer size.
    unsafe { mem::transmute_copy::<*const (), F>(&pointer) }
}

/// Calls the main frame getter, which only reads the pipeline.
fn call_main_frame(fns: &EngineFns, pipeline: Pipeline) -> GameAddr {
    // SAFETY: `main_frame` is the `pipeline_main_frame` match, which takes the pipeline in `rcx`.
    let main_frame = unsafe { function::<MainFrameFn>(fns.main_frame) };
    GameAddr::new(main_frame(pipeline.address().get()))
}

impl GameMemory for RenderCalls<'_> {
    fn writes(&self, address: GameAddr, len: usize) -> WriteDest<'_> {
        game_writes(address, len)
    }
}

impl CreateCalls for RenderCalls<'_> {
    fn main_frame(&mut self, pipeline: Pipeline) -> GameAddr {
        call_main_frame(self.fns, pipeline)
    }

    fn create_player_view(&mut self, manager: GameAddr, frame: RenderFrame) -> GameAddr {
        // SAFETY: `create_player_view` is the `view_create_player` match, which the player manager's
        // vtable slot 1 holds. The feature found the main view in `frame`'s view list before the
        // call.
        let create = unsafe { function::<CreatePlayerViewFn>(self.fns.create_player_view) };
        GameAddr::new(create(manager.get(), frame.address().get()))
    }

    fn pipeline_setup(&mut self, pipeline: Pipeline, view: RenderView) {
        // SAFETY: `pipeline_setup` is the `game_pipeline_setup` match, which the pipeline's vtable
        // slot 88 holds. The feature checked that `view` is a game render view with its own stages.
        let setup = unsafe { function::<PipelineSetupFn>(self.fns.pipeline_setup) };
        setup(pipeline.address().get(), view.address().get());
    }
}

impl GameMemory for GameCalls<'_> {
    fn writes(&self, address: GameAddr, len: usize) -> WriteDest<'_> {
        game_writes(address, len)
    }
}

impl SetUpCalls for GameCalls<'_> {
    fn main_frame(&mut self, pipeline: Pipeline) -> GameAddr {
        call_main_frame(self.fns, pipeline)
    }

    fn set_camera(&mut self, view: RenderView, controller: GameAddr) {
        // SAFETY: `set_camera` is slot 10 of the game render view's vtable, which the feature found
        // as `view`'s first word this frame. `controller` is the main view's `WriteState` controller.
        let set_camera = unsafe { function::<SetCameraFn>(self.fns.set_camera) };
        set_camera(view.address().get(), controller.get(), NO_FLAG);
    }

    fn set_scene(&mut self, view: RenderView, scene: GameAddr) {
        // SAFETY: `set_scene` is the `view_set_scene` match. `scene` is the level's scene, which the
        // feature found equal to the main view's.
        let set_scene = unsafe { function::<SetSceneFn>(self.fns.set_scene) };
        set_scene(view.address().get(), scene.get());
    }

    fn attach_level_atmosphere(
        &mut self,
        view: RenderView,
        id: GameAddr,
        request: &mut i32,
        apply: Apply,
    ) {
        environment(self.fns.attach_level_atmosphere, view, id, request, apply);
    }

    fn attach_level_lighting(
        &mut self,
        view: RenderView,
        id: GameAddr,
        request: &mut i32,
        apply: Apply,
    ) {
        environment(self.fns.attach_level_lighting, view, id, request, apply);
    }

    fn release_id(&mut self, list: GameAddr, id: &mut i32) {
        // SAFETY: `release_id` is the `id_pool_release` match, and `list` the id pool's release list.
        let release = unsafe { function::<ReleaseIdFn>(self.fns.release_id) };
        release(list.get(), id);
    }

    fn set_atmosphere(
        &mut self,
        view: RenderView,
        record: GameAddr,
        request: &mut i32,
        apply: Apply,
    ) {
        environment(self.fns.set_atmosphere, view, record, request, apply);
    }

    fn set_lighting(
        &mut self,
        view: RenderView,
        record: GameAddr,
        request: &mut i32,
        apply: Apply,
    ) {
        environment(self.fns.set_lighting, view, record, request, apply);
    }

    fn set_fog_variant(&mut self, view: RenderView, variant: u8) {
        // SAFETY: `set_fog_variant` is the `view_set_fog_variant` match, which takes a view and a
        // variant byte. `variant` is the fog variant byte read from the main view.
        let set_fog_variant = unsafe { function::<SetFogVariantFn>(self.fns.set_fog_variant) };
        set_fog_variant(view.address().get(), variant);
    }

    fn set_viewport(&mut self, view: RenderView, scale: [f32; 2], offset: [f32; 2]) {
        // SAFETY: `set_viewport` is the `view_set_viewport` match. It reads both pairs before it
        // returns, and they outlive the call.
        let set_viewport = unsafe { function::<SetViewportFn>(self.fns.set_viewport) };
        set_viewport(view.address().get(), &raw const scale, &raw const offset);
    }
}

/// Calls one of the four environment functions, which share one signature, with no blend time and
/// the flag byte of `apply`.
fn environment(
    function_address: usize,
    view: RenderView,
    source: GameAddr,
    request: &mut i32,
    apply: Apply,
) {
    // SAFETY: `function_address` is the match of `view_attach_level_atmosphere`,
    // `view_attach_level_lighting`, `view_set_atmosphere` or `view_set_lighting`, which all take a
    // view, a source pointer, an `i32` pointer, an `f32` and a byte. The feature checked the view's
    // lock bytes and that the source is not null, and read a setter's source record id first.
    let call = unsafe { function::<EnvironmentFn>(function_address) };
    call(view.address().get(), source.get(), request, NO_BLEND, apply.flag());
}
