//! The game state feature's decision: the place a state id names, and whether a read id gets a game
//! state line.

use game_bindings::game_client::StateId;
use ipc_layout::game_state_line::{GameStateLine, Place};

/// The place that `id` names. An id that is none of the measured ids names [`Place::Other`].
pub(crate) const fn place(id: StateId) -> Place {
    match id {
        StateId::MENU => Place::Menu,
        StateId::LEVEL => Place::Level,
        _ => Place::Other,
    }
}

/// The game state line of the read id `read`, or `None` when it equals `logged`, the id of the last
/// game state line.
pub(crate) fn line(logged: Option<StateId>, read: StateId) -> Option<GameStateLine> {
    if logged == Some(read) {
        return None;
    }
    Some(GameStateLine { place: place(read), state_id: read.get() })
}
