//! The pick ray hook's install at the entry of the ray function and the write destination of the
//! cached argument on the caller's stack.

use std::mem::ManuallyDrop;

use game_bindings::render_view::CameraController;
use game_mem::{GameAddr, WriteDest};
use hooks::{HookError, MidHook, MidThunk, Registers};

/// The cached argument's distance from `rsp` at the ray function's entry. The return address, the
/// four register arguments' home slots and the fifth argument come before it.
const CACHED_ARGUMENT: usize = 6 * size_of::<usize>();

/// Installs `thunk` as the mid hook at the ray function's entry `address`.
///
/// `address` must be the one match of the `camera_screen_to_world_ray` signature in the loaded game
/// exe. The hook never drops, so it stays installed for the process life on every path, and no game
/// thread runs a freed stub.
pub(crate) fn install_pick_ray_hook(
    address: GameAddr,
    thunk: MidThunk,
) -> Result<ManuallyDrop<MidHook>, HookError> {
    // SAFETY: the signature's one match is a function entry whose 19 bytes the doc of
    // `CAMERA_SCREEN_TO_WORLD_RAY` lists. SafetyHook's 5-byte jump moves its first 7 bytes and its
    // 14-byte fallback moves all 19. No moved instruction has a RIP-relative operand, and no jump targets a byte
    // past a function's entry. The game's picking
    // calls the function on the game thread (measured in a probe trial), which the game thunk's
    // token requires.
    let hook = unsafe { hooks::install_mid_hook(address, thunk) }?;
    Ok(ManuallyDrop::new(hook))
}

/// The controller that `rcx` holds at the ray function's entry, and the destination of a write to
/// the cached argument, the byte [`CACHED_ARGUMENT`] past `rsp`, which lives for the hook's borrow of
/// `registers`.
/// `None` when that byte's address lies past the end of the address space. Only the pick ray hook
/// passes its registers here.
pub(crate) fn ray_call(registers: &Registers) -> Option<(CameraController, WriteDest<'_>)> {
    let controller = CameraController::new(GameAddr::new(registers.rcx()));
    let argument = GameAddr::new(registers.rsp()).checked_add(CACHED_ARGUMENT)?;
    // SAFETY: at the function's entry `argument` is the sixth argument's slot in the caller's frame
    // on the calling thread's stack, which no Rust reference covers. The destination lives only for
    // the hook, which runs before the function reads the slot.
    let writes = unsafe { WriteDest::new(argument, size_of::<u8>()) };
    Some((controller, writes))
}

#[cfg(test)]
mod tests;
