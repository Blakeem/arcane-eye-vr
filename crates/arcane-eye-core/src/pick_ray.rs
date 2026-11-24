//! The pick ray feature: in the launch mode `vr`, a hook at the entry of the game's ray function,
//! `ScreenToWorldRay`, that gives the ray of an eye camera's proxy from the eye camera that drew the
//! picture.
//!
//! The game's picking calls the ray function once per frame on the game thread with the cached
//! argument 0. [`CAMERA_SCREEN_TO_WORLD_RAY`] states what the function builds the ray from for each
//! cached argument. A proxy copies the game camera's look direction, so with the argument 0 a head
//! turned against the game camera turns the picture and not the ray. A proxy holds the eye camera's
//! inverse view, so the hook sets the argument 1 for a proxy and leaves every other call unchanged.
//! The feature is no
//! feature of the stereo pair (see [`pair`](crate::pair)), so its failure leaves the pair running
//! and the game's picking as without the hook.

use std::mem::ManuallyDrop;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use diag::health::{Feature, Health};
use diag::rate::{Once, RateLimit};
use diag::{once, warn_every};
use game_bindings::render_view::CameraController;
use game_bindings::signatures::CAMERA_SCREEN_TO_WORLD_RAY;
use game_bindings::state;
use game_mem::{GameAddr, WriteDest, WriteError, try_read, try_write};
use hooks::{GameThread, MidHook, Registers, Runs};
use ipc_layout::launch_token::Mode;
use tracing::{Level, info, warn};

use crate::config::Config;
use crate::context::CoreContext;
use crate::eye_camera::proxy;
use crate::install::{GameCode, InstallError};
use crate::raw::pick_ray as raw_pick_ray;

/// The cached argument with which the ray function takes the controller's inverse view.
const FROM_INVERSE_VIEW: u8 = 1;

/// The shortest time between two warnings of a cached argument that was not set.
const LOG_PERIOD: Duration = Duration::from_secs(10);

hooks::game_mid_hook!(
    ray_thunk,
    Feature::PickRay,
    Runs::WhileActive,
    state::core_context::<CoreContext>(),
    on_ray
);

/// Installs the pick ray hook in `code`, the game exe's code, in the launch mode `vr`, and reports
/// the outcome in the health registry and the log. In the mode `desktop-stereo` it installs nothing
/// and logs that. A failure installs nothing and leaves the other features running.
pub(crate) fn install(
    health: &Health,
    code: &GameCode,
    config: Config,
) -> Option<ManuallyDrop<MidHook>> {
    match config.mode {
        Mode::DesktopStereo => {
            info!(mode = config.mode.as_str(), "pick ray hook not installed");
            return None;
        }
        Mode::Vr => {}
    }
    match hook_ray(code) {
        Ok((site, hook)) => {
            health.install_ok(Feature::PickRay);
            info!(address = format_args!("{:#x}", site.get()), "pick ray hook installed");
            Some(hook)
        }
        Err(error) => {
            health.install_failed(Feature::PickRay);
            warn!(error = %error, "pick ray hook not installed");
            None
        }
    }
}

fn hook_ray(code: &GameCode) -> Result<(GameAddr, ManuallyDrop<MidHook>), InstallError> {
    let site = code.locate(&CAMERA_SCREEN_TO_WORLD_RAY)?;
    let hook = raw_pick_ray::install_pick_ray_hook(site, ray_thunk)?;
    Ok((site, hook))
}

/// The hook's handler, at the ray function's entry. `rcx` holds the controller.
fn on_ray(context: &CoreContext, _thread: &GameThread, registers: &mut Registers) {
    let Some((controller, cached)) = raw_pick_ray::ray_call(registers) else { return };
    context.pick_ray.on_call(controller, cached);
}

/// What a call's controller is, for the cached argument.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Controller {
    /// An eye camera's proxy, which holds the eye camera's inverse view.
    Proxy,
    /// Any other controller, or one whose vtable or slot 9 cannot be read.
    Other,
}

impl Controller {
    /// The kind of `controller`, from guarded reads of its vtable pointer and that vtable's slot 9.
    fn of(controller: CameraController) -> Self {
        if controller.world_translate_fn() == Some(proxy::world_translate_fn()) {
            Self::Proxy
        } else {
            Self::Other
        }
    }
}

/// The cached argument a call whose controller is `controller` runs with, or `None` when the call
/// runs with the argument its caller passed.
const fn cached_argument(controller: Controller) -> Option<u8> {
    match controller {
        Controller::Proxy => Some(FROM_INVERSE_VIEW),
        Controller::Other => None,
    }
}

/// Why a call runs with the argument its caller passed, although its controller is a proxy.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
enum ArgumentFault {
    #[error("the cached argument at {address:#x} could not be read")]
    Unreadable { address: usize },
    #[error(transparent)]
    Unwritten(#[from] WriteError),
}

const _: () = assert!(size_of::<ArgumentFault>() <= 16, "a per-frame error holds at most 16 bytes");

/// What a call's handler did to its cached argument.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Change {
    Unchanged,
    Set,
}

/// The feature's state in the core context: the log state that the ray function's calls share.
#[derive(Debug, Default)]
pub(crate) struct PickRay {
    logs: Mutex<Logs>,
}

#[derive(Debug, Default)]
struct Logs {
    first_change: Once,
    fault: RateLimit,
}

impl PickRay {
    /// Gives the call whose controller is `controller` the cached argument that [`cached_argument`]
    /// names, writing it to `cached`. The first call that changes the argument logs one line, and a
    /// fault logs a rate-limited warning, after which the call runs with the argument its caller
    /// passed.
    pub(crate) fn on_call(&self, controller: CameraController, cached: WriteDest<'_>) {
        match set_argument(controller, cached) {
            Ok(Change::Unchanged) => {}
            Ok(Change::Set) => {
                let mut logs = self.lock();
                once!(
                    logs.first_change,
                    Level::INFO,
                    controller = format_args!("{:#x}", controller.address().get()),
                    "pick ray takes a proxy's ray from the eye camera's inverse view"
                );
            }
            Err(fault) => {
                let mut logs = self.lock();
                warn_every!(
                    logs.fault,
                    LOG_PERIOD,
                    controller = format_args!("{:#x}", controller.address().get()),
                    fault = %fault,
                    "pick ray did not set a proxy's cached argument, so its ray follows the game camera"
                );
            }
        }
    }

    /// The log state. A panic that held the lock switched the feature off, so no later call
    /// reaches it.
    fn lock(&self) -> MutexGuard<'_, Logs> {
        self.logs.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Writes the argument [`cached_argument`] names for `controller` to `cached` when the argument
/// there differs. A call with no such argument leaves `cached` unread.
fn set_argument(
    controller: CameraController,
    cached: WriteDest<'_>,
) -> Result<Change, ArgumentFault> {
    let Some(wanted) = cached_argument(Controller::of(controller)) else {
        return Ok(Change::Unchanged);
    };
    let address = cached.start();
    let Some(argument) = try_read::<u8>(address) else {
        return Err(ArgumentFault::Unreadable { address: address.get() });
    };
    if wanted == argument {
        return Ok(Change::Unchanged);
    }
    try_write(cached, wanted)?;
    Ok(Change::Set)
}

#[cfg(test)]
mod tests;
