//! State id reads against a game client global, client and state machine built in the test
//! process's memory.

use std::hint::black_box;

use game_mem::GameAddr;

use super::{ClientGlobal, ClientState, StateId};
use crate::offsets::{GAME_CLIENT_STATE_MACHINE, STATE_MACHINE_STATE_ID};

/// Windows reserves the lowest 64 KiB of every process, so no read there succeeds.
const UNMAPPED: usize = 0x10;

fn address_of<T>(values: &[T]) -> usize {
    values.as_ptr().expose_provenance()
}

/// A global that points to a client, whose state machine pointer and state id the test sets.
struct TestClient {
    global: Vec<usize>,
    client: Vec<u8>,
    _machine: Vec<u8>,
}

impl TestClient {
    /// A client whose state machine holds `id`.
    fn new(id: u32) -> Self {
        let mut machine = vec![0_u8; 0x20];
        machine[STATE_MACHINE_STATE_ID..][..4].copy_from_slice(&id.to_ne_bytes());
        let machine = black_box(machine);
        let mut client = vec![0_u8; 0x100];
        let machine_address = address_of(&machine).to_ne_bytes();
        client[GAME_CLIENT_STATE_MACHINE..][..8].copy_from_slice(&machine_address);
        let client = black_box(client);
        let global = black_box(vec![address_of(&client)]);
        Self { global, client, _machine: machine }
    }

    fn set_client(&mut self, client: usize) {
        self.global[0] = black_box(client);
    }

    fn set_machine(&mut self, machine: usize) {
        self.client[GAME_CLIENT_STATE_MACHINE..][..8].copy_from_slice(&machine.to_ne_bytes());
    }

    fn state(&self) -> Option<ClientState> {
        ClientGlobal::new(GameAddr::new(address_of(&self.global))).state()
    }
}

#[test]
fn a_client_yields_the_state_id_of_its_state_machine() {
    for id in [6, 7, 0x12, 0x0A, u32::MAX] {
        let test = TestClient::new(id);
        assert_eq!(test.state(), Some(ClientState::Id(StateId::new(id))), "{id:#x}");
    }
}

#[test]
fn a_null_client_pointer_yields_no_state_machine() {
    let mut test = TestClient::new(0x12);
    test.set_client(0);
    assert_eq!(test.state(), Some(ClientState::NoStateMachine));
}

#[test]
fn a_null_state_machine_pointer_yields_no_state_machine() {
    let mut test = TestClient::new(0x12);
    test.set_machine(0);
    assert_eq!(test.state(), Some(ClientState::NoStateMachine));
}

#[test]
fn an_unmapped_global_client_or_state_machine_yields_nothing() {
    assert_eq!(ClientGlobal::new(GameAddr::new(UNMAPPED)).state(), None);
    let mut unmapped_client = TestClient::new(7);
    unmapped_client.set_client(UNMAPPED);
    assert_eq!(unmapped_client.state(), None);
    let mut unmapped_machine = TestClient::new(7);
    unmapped_machine.set_machine(UNMAPPED);
    assert_eq!(unmapped_machine.state(), None);
}
