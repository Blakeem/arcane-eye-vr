//! Starts and stops the game with a launch token. The launch app and the trial runner use it.
//!
//! The Steam start opens the game's Steam URL and needs a Play click in the Larian launcher. The direct
//! start runs `bin\bg3_dx11.exe` itself. `reload.rs` asks the running game's core to read the settings
//! files again. The trial runner also runs on a desktop of its own (`desktop.rs`), moves the cursor
//! in the game window (`cursor.rs`) and copies another process's window from the screen
//! (`window_copy.rs`). The headset screen sends the game its mouse input (`mouse.rs`) and its
//! camera turn and pan keys (`camera_key.rs`), and shows and clicks the Larian launcher's window
//! (`launcher_window.rs`). The pure decisions are in `decide.rs` and `vdf.rs`. `raw.rs` and
//! the files under `raw\` hold every unsafe Windows call.

/// Baldur's Gate 3's Steam app id, as a literal so `concat!` can build the names that hold it.
macro_rules! steam_app_id {
    () => {
        "1086940"
    };
}

pub mod camera_key;
pub mod checks;
pub mod cursor;
mod decide;
pub mod desktop;
pub mod direct;
pub mod game_dir;
pub mod key;
pub mod launcher_window;
pub mod mouse;
mod raw;
pub mod recycle;
pub mod registry;
pub mod reload;
pub mod status;
pub mod steam;
pub mod stop;
pub mod token;
mod vdf;
pub mod window_copy;

/// A temp folder for test `name` of this crate.
#[cfg(test)]
fn test_dir(name: &str) -> test_support::TestDir {
    test_support::TestDir::in_temp("arcane-eye-launch", name).expect("create the test folder")
}

/// A launch token of the mode `vr` whose status word holds `status`, as the game writes it.
#[cfg(test)]
fn token_with_status(
    status: u32,
) -> Result<ipc_layout::launch_token::LaunchToken, ipc_layout::launch_token::TokenError> {
    use ipc_layout::launch_token::{LaunchToken, Mode, STATUS_OFFSET};
    let mut bytes = LaunchToken::new(Mode::Vr, 1, 2).encode();
    bytes[STATUS_OFFSET..STATUS_OFFSET + 4].copy_from_slice(&status.to_le_bytes());
    LaunchToken::decode(&bytes)
}
