//! The view kind: whether a frame's proxies place the eyes by the head pose, so the launch app
//! shows the frame around the player, or on the head controller's right axis, so it shows the frame
//! on the screen.

use game_bindings::game_client::StateId;
use ipc_layout::launch_token::Mode;
use tracing::info;

use super::Rays;

/// The cinematic byte of gameplay. A dialogue or a cutscene sets the byte to 1.
const NO_CINEMATIC: u8 = 0;

/// The value the view kind line gives a read that failed or never ran.
const NO_READ: i64 = -1;

/// Where the launch app shows a frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ViewKind {
    /// Around the player: the head pose places the eyes, and the frame's tag names that pose.
    World,
    /// On the screen: the eyes sit on the head controller's right axis as in the launch mode
    /// `desktop-stereo`, and the frame's tag names no pose.
    Screen,
}

impl ViewKind {
    /// The name the `vr view` line and the view kind line give it.
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::World => "world",
            Self::Screen => "screen",
        }
    }
}

/// The newest reads of the game's state at a Present call. Each is `None` for a read that failed,
/// found nothing to read or belongs to a feature that does not run.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct GameRead {
    /// The game client's state id.
    pub(crate) state_id: Option<StateId>,
    /// The main view's cinematic byte.
    pub(crate) cinematic: Option<u8>,
}

/// The view kind of the next frame's proxies: world only in the launch mode `vr` while the view
/// camera rewrites the corner rays, the game runs a level and the main view shows no cinematic,
/// and screen in every other case.
pub(crate) fn decide(mode: Mode, rays: Rays, game: GameRead) -> ViewKind {
    let world = mode == Mode::Vr
        && rays == Rays::Rewritten
        && game.state_id == Some(StateId::LEVEL)
        && game.cinematic == Some(NO_CINEMATIC);
    if world { ViewKind::World } else { ViewKind::Screen }
}

/// Logs the line of a change of the decided view kind to `kind` at the reads `game`, with -1 for a
/// read that failed or never ran.
pub(super) fn log_change(kind: ViewKind, game: GameRead) {
    let state_id = game.state_id.map_or(NO_READ, |id| i64::from(id.get()));
    let cinematic = game.cinematic.map_or(NO_READ, i64::from);
    // Only a change of the decided kind reaches this line, so it needs no rate-limit macro.
    info!(view = kind.as_str(), state_id, cinematic, "view kind changed");
}

#[cfg(test)]
mod tests;
