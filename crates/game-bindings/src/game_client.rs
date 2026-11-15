//! The typed, guarded read of the game client's state id, which says where the game is.
//!
//! The global that [`G_GAME_CLIENT`](crate::signatures::G_GAME_CLIENT) yields points to the game
//! client, and the client points to its state machine. Each read goes through `game-mem`'s guarded
//! copy, so a wrong address yields `None` and never a fault.

use game_mem::{GameAddr, try_read};

use crate::offsets::{GAME_CLIENT_STATE_MACHINE, STATE_MACHINE_STATE_ID};

/// A state id of the game client's state machine. The ids are the values of the Script Extender's
/// `ecl::GameState` enum. Only the ids of the constants were measured.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StateId(u32);

impl StateId {
    /// The menu state: the "Press any key to continue" screen and the main menu after it.
    pub const MENU: Self = Self(7);
    /// A running level.
    pub const LEVEL: Self = Self(0x12);

    pub const fn new(id: u32) -> Self {
        Self(id)
    }

    pub const fn get(self) -> u32 {
        self.0
    }
}

/// The game client global, by its address in the game exe. It holds the game client pointer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClientGlobal(GameAddr);

/// What the game client's state machine holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClientState {
    /// The state machine is in this state.
    Id(StateId),
    /// The client pointer or the state machine pointer is null, so no state id exists.
    NoStateMachine,
}

impl ClientGlobal {
    pub const fn new(address: GameAddr) -> Self {
        Self(address)
    }

    pub const fn address(self) -> GameAddr {
        self.0
    }

    /// The state the game client's state machine holds, or `None` when a pointer or the state id
    /// cannot be read.
    pub fn state(self) -> Option<ClientState> {
        let client = try_read::<usize>(self.0)?;
        if client == 0 {
            return Some(ClientState::NoStateMachine);
        }
        let machine =
            try_read::<usize>(GameAddr::new(client).checked_add(GAME_CLIENT_STATE_MACHINE)?)?;
        if machine == 0 {
            return Some(ClientState::NoStateMachine);
        }
        let id = try_read::<u32>(GameAddr::new(machine).checked_add(STATE_MACHINE_STATE_ID)?)?;
        Some(ClientState::Id(StateId(id)))
    }
}

#[cfg(test)]
mod tests;
