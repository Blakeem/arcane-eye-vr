//! The checks before a start: SteamVR is the active OpenXR runtime, and the game is not running.

use tracing::warn;

use crate::decide;
use crate::registry::{Hive, RegistryError, RegistryValue};

/// The active OpenXR runtime's manifest path, which the runtime's installer writes.
const ACTIVE_RUNTIME: RegistryValue = RegistryValue {
    hive: Hive::LocalMachine,
    key: r"SOFTWARE\Khronos\OpenXR\1",
    name: "ActiveRuntime",
};

/// 1 while Steam runs the game. Steam writes it, and Valve does not document it.
const GAME_RUNNING: RegistryValue = RegistryValue {
    hive: Hive::CurrentUser,
    key: concat!(r"Software\Valve\Steam\Apps\", steam_app_id!()),
    name: "Running",
};

/// Why the SteamVR runtime check failed.
#[derive(Debug, thiserror::Error)]
pub enum RuntimeError {
    #[error(transparent)]
    Read(#[from] RegistryError),
    #[error("the active OpenXR runtime is {}, not SteamVR", active.as_deref().unwrap_or("not set"))]
    NotSteamVr { active: Option<String> },
}

/// Steam reports that the game runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("Steam reports that the game is running")]
pub struct GameRunning;

/// Checks that SteamVR is the active OpenXR runtime.
///
/// # Errors
///
/// Returns [`RuntimeError::Read`] when the registry read fails and [`RuntimeError::NotSteamVr`] when
/// another runtime or no runtime is active.
pub fn check_runtime() -> Result<(), RuntimeError> {
    decide::check_runtime(ACTIVE_RUNTIME.read_string()?)
}

/// Fails when Steam reports the game running. A failed read is a warn line.
///
/// # Errors
///
/// Returns [`GameRunning`] when Steam's `Running` value for the game is nonzero.
pub fn check_game_not_running() -> Result<(), GameRunning> {
    let running = GAME_RUNNING.read_dword();
    if let Err(error) = &running {
        warn!(error = %error, "could not read whether the game runs, so the launch goes ahead");
    }
    decide::check_not_running(&running)
}

/// Whether Steam reports the game running. A failed read logs nothing, so the caller words its own
/// line.
///
/// # Errors
///
/// Returns the [`RegistryError`] of a failed read of Steam's `Running` value.
pub fn game_running() -> Result<bool, RegistryError> {
    Ok(decide::reports_running(GAME_RUNNING.read_dword()?))
}
