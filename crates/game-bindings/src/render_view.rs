//! Typed, guarded reads and writes of an engine render view and of the camera controller its
//! `ReadState` names, and the view fields the second view's recipe reads and writes.
//!
//! Each read and write goes through `game-mem`'s guarded copy, so a wrong address yields a failure
//! value and never a fault. A read checks no type, so a right address is the caller's part. A write
//! goes to the view's [`WriteDest`], which the caller's unsafe code constructs.

use game_mem::{GameAddr, WriteDest, WriteError, try_read, try_read_array, try_write};

use crate::camera::PerCamera;
use crate::offsets::{
    CONTROLLER_SIZE, CONTROLLER_VIEW, CONTROLLER_VTABLE_SLOTS, CONTROLLER_WORLD_TRANSLATE_SLOT,
    ENVIRONMENT_RECORD_ID, PER_CAMERA_FAR, PER_CAMERA_NEAR, PER_CAMERA_POSITION,
    PER_CAMERA_PROJECTION, PER_CAMERA_VIEW, SCENE_ACTIVE, VIEW_ATMOSPHERE, VIEW_ATMOSPHERE_LOCK,
    VIEW_ATMOSPHERE_REQUEST, VIEW_ATMOSPHERE_TARGET, VIEW_JOB_CONTEXT, VIEW_LAST_SEEN_CONTROLLER,
    VIEW_LIGHTING, VIEW_LIGHTING_LOCK, VIEW_LIGHTING_REQUEST, VIEW_LIGHTING_TARGET, VIEW_OCCLUSION,
    VIEW_PER_CAMERA, VIEW_READ_STATE_CONTROLLER, VIEW_SCENE, VIEW_SPLIT_INDEX, VIEW_SPLIT_SIDE,
    VIEW_STAGE_ARRAY, VIEW_STAGE_OWNER, VIEW_WRITE_STATE_CONTROLLER, VIEW_WRITE_STATE_ENABLED,
};

/// A render view, by its address in the game process.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RenderView(GameAddr);

/// A camera controller, by its address in the game process.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CameraController(GameAddr);

/// What a render view's `ReadState` controller pointer holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadState {
    /// A null pointer, as the special view holds.
    Null,
    Controller(CameraController),
}

/// A byte copy of a camera controller object, taken in one guarded read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ControllerCopy([u8; CONTROLLER_SIZE]);

/// The index of the view matrix's first `f32` in a controller's bytes.
const VIEW_WORD: usize = CONTROLLER_VIEW / size_of::<f32>();

// `WRITE_LEN` ends at the last-seen field, so every other written field must come before it.
const _: () = assert!(VIEW_READ_STATE_CONTROLLER < VIEW_LAST_SEEN_CONTROLLER);
const _: () = assert!(VIEW_WRITE_STATE_ENABLED < VIEW_LAST_SEEN_CONTROLLER);
const _: () = assert!(VIEW_SPLIT_INDEX < VIEW_LAST_SEEN_CONTROLLER);
const _: () = assert!(VIEW_SPLIT_SIDE + size_of::<i32>() <= VIEW_LAST_SEEN_CONTROLLER);

/// The pointers that tell a render view's own stage objects from another view's. A null pointer
/// reads as address 0.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ViewParts {
    pub vtable: GameAddr,
    /// The owner view of the view's stage group.
    pub stage_owner: GameAddr,
    pub stage_array: GameAddr,
    pub job_context: GameAddr,
}

/// The active record id, the target record and the request id of a render view's atmosphere or
/// lighting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Environment {
    /// The active record's id, or `None` when the view has no active record.
    pub active_id: Option<i32>,
    /// The target record, which the setter copies its source into, or address 0 when the view has
    /// none.
    pub target: GameAddr,
    /// The target record's id, or `None` when the target record is null.
    pub target_id: Option<i32>,
    /// The view's request id, -1 when it names no request.
    pub request: i32,
}

/// What a render view renders and the bytes that gate the setters that change it. A null pointer
/// reads as address 0.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ViewContent {
    /// The `WriteState` camera controller.
    pub controller: GameAddr,
    pub scene: GameAddr,
    /// The scene's active byte, or 0 when the scene is null.
    pub scene_active: u8,
    pub atmosphere: Environment,
    pub lighting: Environment,
    pub split_index: u8,
    pub atmosphere_lock: u8,
    pub lighting_lock: u8,
}

impl RenderView {
    /// The bytes from a view's start that hold each field the view's writes fill.
    pub const WRITE_LEN: usize = VIEW_LAST_SEEN_CONTROLLER + size_of::<usize>();

    pub const fn new(address: GameAddr) -> Self {
        Self(address)
    }

    pub const fn address(self) -> GameAddr {
        self.0
    }

    /// What the view's `ReadState` controller pointer holds, or `None` when it cannot be read.
    pub fn read_state(self) -> Option<ReadState> {
        let pointer = try_read::<usize>(self.0.checked_add(VIEW_READ_STATE_CONTROLLER)?)?;
        let state = match pointer {
            0 => ReadState::Null,
            _ => ReadState::Controller(CameraController(GameAddr::new(pointer))),
        };
        Some(state)
    }

    /// The camera controller the view's `ReadState` points to, or `None` when that pointer is null or
    /// cannot be read.
    pub fn read_state_controller(self) -> Option<CameraController> {
        match self.read_state()? {
            ReadState::Controller(controller) => Some(controller),
            ReadState::Null => None,
        }
    }

    /// Points the `ReadState` of the view whose first [`RenderView::WRITE_LEN`] bytes are `view` at
    /// `controller`. The engine's frame sync points it back at the `WriteState` controller at the
    /// frame's end.
    ///
    /// # Errors
    ///
    /// Returns the [`WriteError`] of a field that cannot be written or lies outside `view`.
    pub fn write_read_state(
        view: WriteDest<'_>,
        controller: CameraController,
    ) -> Result<(), WriteError> {
        write_pointer(view, VIEW_READ_STATE_CONTROLLER, controller)
    }

    /// Sets the controller pointer that the view whose first [`RenderView::WRITE_LEN`] bytes are
    /// `view` saw last to `controller`. Begin-recording then sets no camera cut when the `ReadState`
    /// controller is `controller`.
    ///
    /// # Errors
    ///
    /// Returns the [`WriteError`] of a field that cannot be written or lies outside `view`.
    pub fn write_last_seen_controller(
        view: WriteDest<'_>,
        controller: CameraController,
    ) -> Result<(), WriteError> {
        write_pointer(view, VIEW_LAST_SEEN_CONTROLLER, controller)
    }

    /// The controller pointer the view saw last, null included, or `None` when it cannot be read.
    pub fn last_seen_controller(self) -> Option<GameAddr> {
        try_read::<usize>(self.0.checked_add(VIEW_LAST_SEEN_CONTROLLER)?).map(GameAddr::new)
    }

    /// The view's `PerCamera` block 0, or `None` when any field of it cannot be read.
    pub fn per_camera(self) -> Option<PerCamera> {
        let block = self.0.checked_add(VIEW_PER_CAMERA)?;
        Some(PerCamera {
            view: try_read_array(block.checked_add(PER_CAMERA_VIEW)?)?,
            projection: try_read_array(block.checked_add(PER_CAMERA_PROJECTION)?)?,
            near: try_read(block.checked_add(PER_CAMERA_NEAR)?)?,
            far: try_read(block.checked_add(PER_CAMERA_FAR)?)?,
            position: try_read_array(block.checked_add(PER_CAMERA_POSITION)?)?,
        })
    }

    /// The view's vtable pointer, or `None` when it cannot be read.
    pub fn vtable(self) -> Option<GameAddr> {
        try_read::<usize>(self.0).map(GameAddr::new)
    }

    /// The view's stage objects, or `None` when any pointer cannot be read.
    pub fn parts(self) -> Option<ViewParts> {
        Some(ViewParts {
            vtable: self.vtable()?,
            stage_owner: self.pointer(VIEW_STAGE_OWNER)?,
            stage_array: self.pointer(VIEW_STAGE_ARRAY)?,
            job_context: self.pointer(VIEW_JOB_CONTEXT)?,
        })
    }

    /// What the view renders, or `None` when any field cannot be read.
    pub fn content(self) -> Option<ViewContent> {
        let scene = self.pointer(VIEW_SCENE)?;
        let scene_active = match scene.get() {
            0 => 0,
            _ => try_read::<u8>(scene.checked_add(SCENE_ACTIVE)?)?,
        };
        Some(ViewContent {
            controller: self.pointer(VIEW_WRITE_STATE_CONTROLLER)?,
            scene,
            scene_active,
            atmosphere: self.environment(
                VIEW_ATMOSPHERE,
                VIEW_ATMOSPHERE_TARGET,
                VIEW_ATMOSPHERE_REQUEST,
            )?,
            lighting: self.environment(
                VIEW_LIGHTING,
                VIEW_LIGHTING_TARGET,
                VIEW_LIGHTING_REQUEST,
            )?,
            split_index: try_read::<u8>(self.0.checked_add(VIEW_SPLIT_INDEX)?)?,
            atmosphere_lock: try_read::<u8>(self.0.checked_add(VIEW_ATMOSPHERE_LOCK)?)?,
            lighting_lock: try_read::<u8>(self.0.checked_add(VIEW_LIGHTING_LOCK)?)?,
        })
    }

    /// The view's occlusion object, address 0 included, or `None` when the pointer cannot be read.
    pub fn occlusion(self) -> Option<GameAddr> {
        self.pointer(VIEW_OCCLUSION)
    }

    /// Sets the split-screen index of the view whose first [`RenderView::WRITE_LEN`] bytes are `view`.
    ///
    /// # Errors
    ///
    /// Returns the [`WriteError`] of a field that cannot be written or lies outside `view`.
    pub fn write_split_index(view: WriteDest<'_>, index: u8) -> Result<(), WriteError> {
        try_write(view.offset(VIEW_SPLIT_INDEX)?, index)
    }

    /// Sets the `WriteState` Enabled byte and the split-screen side of the view whose first
    /// [`RenderView::WRITE_LEN`] bytes are `view`, as the engine's viewport system does per player
    /// view.
    ///
    /// # Errors
    ///
    /// Returns the [`WriteError`] of a field that cannot be written or lies outside `view`. The first
    /// field may then hold the new value.
    pub fn write_enabled_and_side(
        view: WriteDest<'_>,
        enabled: u8,
        side: i32,
    ) -> Result<(), WriteError> {
        try_write(view.offset(VIEW_WRITE_STATE_ENABLED)?, enabled)?;
        try_write(view.offset(VIEW_SPLIT_SIDE)?, side)
    }

    fn pointer(self, offset: usize) -> Option<GameAddr> {
        try_read::<usize>(self.0.checked_add(offset)?).map(GameAddr::new)
    }

    fn environment(
        self,
        active_offset: usize,
        target_offset: usize,
        request_offset: usize,
    ) -> Option<Environment> {
        let (_, active_id) = self.environment_record(active_offset)?;
        let (target, target_id) = self.environment_record(target_offset)?;
        let request = try_read::<i32>(self.0.checked_add(request_offset)?)?;
        Some(Environment { active_id, target, target_id, request })
    }

    /// The environment record pointer at `offset` and the record's id, `None` for a null record.
    fn environment_record(self, offset: usize) -> Option<(GameAddr, Option<i32>)> {
        let record = self.pointer(offset)?;
        let id = match record.get() {
            0 => None,
            _ => Some(try_read::<i32>(record.checked_add(ENVIRONMENT_RECORD_ID)?)?),
        };
        Some((record, id))
    }
}

fn write_pointer(
    view: WriteDest<'_>,
    offset: usize,
    controller: CameraController,
) -> Result<(), WriteError> {
    try_write(view.offset(offset)?, controller.0.get())
}

impl CameraController {
    pub const fn new(address: GameAddr) -> Self {
        Self(address)
    }

    pub const fn address(self) -> GameAddr {
        self.0
    }

    /// The address of the function in the controller's vtable slot 9, `GetWorldTranslate` for the
    /// gameplay camera controller, or `None` when the vtable or the slot cannot be read.
    pub fn world_translate_fn(self) -> Option<GameAddr> {
        let vtable = try_read::<usize>(self.0)?;
        let slot = GameAddr::new(vtable).checked_add(CONTROLLER_WORLD_TRANSLATE_SLOT)?;
        try_read::<usize>(slot).map(GameAddr::new)
    }

    /// A copy of the controller object's bytes, or `None` when any of them cannot be read.
    pub fn copy(self) -> Option<ControllerCopy> {
        try_read_array(self.0).map(ControllerCopy)
    }

    /// The first [`CONTROLLER_VTABLE_SLOTS`] slots of the controller's vtable, or `None` when the
    /// vtable pointer or any slot cannot be read.
    pub fn vtable_slots(self) -> Option<[usize; CONTROLLER_VTABLE_SLOTS]> {
        let vtable = try_read::<usize>(self.0)?;
        try_read_array(GameAddr::new(vtable))
    }
}

impl ControllerCopy {
    pub const fn bytes(&self) -> &[u8; CONTROLLER_SIZE] {
        &self.0
    }

    /// The copied world-to-camera view matrix, 16 `f32` in column-major order.
    pub fn view(&self) -> [f32; 16] {
        let (words, _) = self.0.as_chunks::<4>();
        let mut view = [0.0; 16];
        for (value, word) in view.iter_mut().zip(words.iter().skip(VIEW_WORD)) {
            *value = f32::from_ne_bytes(*word);
        }
        view
    }
}

#[cfg(test)]
mod tests;
