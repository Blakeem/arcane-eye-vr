//! The game state feature's decision, its log lines and its newest read against a game client in
//! test memory, and its install.

use std::hint::black_box;

use diag::health::{Feature, FeatureState, Health};
use game_bindings::game_client::{ClientGlobal, StateId};
use game_bindings::offsets::{GAME_CLIENT_STATE_MACHINE, STATE_MACHINE_STATE_ID};
use game_bindings::state::arm_alloc_guard;
use game_mem::GameAddr;
use ipc_layout::game_state_line::{self, GameStateLine, Place};

use super::decide::{line, place};
use super::{GameStateWatch, install};
use crate::install::GameCode;
use crate::logged;

/// Windows reserves the lowest 64 KiB of every process, so no read there succeeds.
const UNMAPPED: usize = 0x10;

/// The message of a failed or skipped install.
const NOT_INSTALLED: &str = "game state read not installed";

/// The message of a failed read.
const READ_FAILED: &str = "game state read of game memory failed for this frame";

fn address_of<T>(values: &[T]) -> usize {
    values.as_ptr().expose_provenance()
}

/// A game client global, client and state machine in test memory, with state id 0.
struct TestClient {
    global: Vec<usize>,
    client: Vec<u8>,
    machine: Vec<u8>,
}

impl TestClient {
    fn new() -> Self {
        let machine = black_box(vec![0_u8; 0x20]);
        let mut client = vec![0_u8; 0x100];
        let machine_address = address_of(&machine).to_ne_bytes();
        client[GAME_CLIENT_STATE_MACHINE..][..8].copy_from_slice(&machine_address);
        let client = black_box(client);
        let global = black_box(vec![address_of(&client)]);
        Self { global, client, machine }
    }

    fn watch(&self) -> GameStateWatch {
        GameStateWatch::new(ClientGlobal::new(GameAddr::new(address_of(&self.global))))
    }

    fn set_id(&mut self, id: u32) {
        self.machine[STATE_MACHINE_STATE_ID..][..4].copy_from_slice(&id.to_ne_bytes());
        black_box(&self.machine);
    }

    fn set_client(&mut self, client: usize) {
        self.global[0] = client;
        black_box(&self.global);
    }

    fn client_address(&self) -> usize {
        address_of(&self.client)
    }

    fn set_machine(&mut self, machine: usize) {
        self.client[GAME_CLIENT_STATE_MACHINE..][..8].copy_from_slice(&machine.to_ne_bytes());
        black_box(&self.client);
    }
}

/// A registry with the game state read running.
fn running() -> Health {
    let health = Health::new();
    health.install_ok(Feature::GameState);
    health
}

/// The game state lines of `text`, each with its place and state id.
fn game_states(text: &str) -> Vec<GameStateLine> {
    let parsed = text.lines().map(|line| game_state_line::parse_line(line).expect("a valid line"));
    parsed.flatten().collect()
}

const fn state(place: Place, state_id: u32) -> GameStateLine {
    GameStateLine { place, state_id }
}

#[test]
fn the_measured_ids_name_their_places_and_every_other_id_names_other() {
    assert_eq!(place(StateId::new(7)), Place::Menu);
    assert_eq!(place(StateId::new(0x12)), Place::Level);
    for id in [0, 6, 0x09, 0x0A, 0x0B, 0x0C, 0x11, 0x16, 0x17, u32::MAX] {
        assert_eq!(place(StateId::new(id)), Place::Other, "{id:#x}");
    }
}

#[test]
fn the_first_id_and_each_changed_id_get_a_line_and_an_equal_id_none() {
    let load_menu = StateId::new(6);
    assert_eq!(line(None, load_menu), Some(state(Place::Other, 6)), "the first id");
    assert_eq!(line(Some(load_menu), load_menu), None, "an equal id");
    let menu = line(Some(load_menu), StateId::new(7));
    assert_eq!(menu, Some(state(Place::Menu, 7)), "a changed id");
    let loading = line(Some(StateId::new(0x12)), StateId::new(0x0A));
    assert_eq!(loading, Some(state(Place::Other, 0x0A)), "a changed id of no measured place");
}

#[test]
fn each_change_of_the_state_id_logs_one_line_that_parses_back_to_its_place_and_id() {
    let mut memory = TestClient::new();
    let mut watch = memory.watch();
    let ((), text) = logged("game-state-changes", || {
        for id in [0x16, 0x16, 6, 6, 7, 0x0A, 0x12, 0x12] {
            memory.set_id(id);
            watch.on_frame();
        }
    });
    let expected = [
        state(Place::Other, 0x16),
        state(Place::Other, 6),
        state(Place::Menu, 7),
        state(Place::Other, 0x0A),
        state(Place::Level, 0x12),
    ];
    assert_eq!(game_states(&text), expected, "{text}");
    let lines: Vec<&str> = text.lines().collect();
    let [_, _, menu, ..] = lines.as_slice() else { panic!("five lines: {text}") };
    assert!(menu.contains(" INFO  "), "{menu}");
    assert!(menu.ends_with(" msg=\"game state\" place=menu state_id=7"), "{menu}");
}

#[test]
fn a_null_client_or_state_machine_pointer_logs_nothing_and_keeps_the_last_id() {
    let mut memory = TestClient::new();
    let mut watch = memory.watch();
    let client = memory.client_address();
    let ((), text) = logged("game-state-null", || {
        memory.set_id(6);
        watch.on_frame();
        memory.set_client(0);
        watch.on_frame();
        memory.set_client(client);
        memory.set_machine(0);
        watch.on_frame();
    });
    assert_eq!(game_states(&text), [state(Place::Other, 6)], "{text}");
    assert_eq!(text.lines().count(), 1, "{text}");
}

#[test]
fn a_failed_read_logs_one_rate_limited_warning_and_keeps_the_last_id() {
    let mut memory = TestClient::new();
    let mut watch = memory.watch();
    let client = memory.client_address();
    let ((), text) = logged("game-state-unreadable", || {
        memory.set_id(7);
        watch.on_frame();
        memory.set_client(UNMAPPED);
        watch.on_frame();
        watch.on_frame();
        memory.set_client(client);
        watch.on_frame();
        memory.set_id(0x12);
        watch.on_frame();
    });
    assert_eq!(game_states(&text), [state(Place::Menu, 7), state(Place::Level, 0x12)]);
    let warnings: Vec<&str> = text.lines().filter(|line| line.contains(READ_FAILED)).collect();
    let [warning] = warnings.as_slice() else { panic!("one warning: {text}") };
    let global = format!(" global={:#x}", address_of(&memory.global));
    assert!(warning.contains(" WARN  ") && warning.ends_with(&global), "{warning}");
}

#[test]
fn the_newest_read_is_the_read_id_and_none_after_a_null_pointer_or_a_failed_read() {
    let mut memory = TestClient::new();
    let mut watch = memory.watch();
    let client = memory.client_address();
    let health = running();
    assert_eq!(watch.newest(&health), None, "before the first read");
    let mut newest = Vec::new();
    let ((), _text) = logged("game-state-newest", || {
        memory.set_id(0x12);
        watch.on_frame();
        newest.push(watch.newest(&health));
        memory.set_machine(0);
        watch.on_frame();
        newest.push(watch.newest(&health));
        memory.set_client(UNMAPPED);
        watch.on_frame();
        newest.push(watch.newest(&health));
        memory.set_client(client);
        memory.set_id(7);
        let machine = address_of(&memory.machine);
        memory.set_machine(machine);
        watch.on_frame();
        newest.push(watch.newest(&health));
    });
    let expected = [Some(StateId::new(0x12)), None, None, Some(StateId::new(7))];
    assert_eq!(newest, expected, "a level, no state machine, a failed read, the title screen");
}

#[test]
fn a_feature_latched_off_gives_no_newest_read_after_a_level_read() {
    let mut memory = TestClient::new();
    let mut watch = memory.watch();
    let health = running();
    memory.set_id(18);
    let ((), _text) = logged("game-state-latched", || watch.on_frame());
    let level = watch.newest(&health);

    health.panic_caught(Feature::GameState);

    assert_eq!(level, Some(StateId::LEVEL), "a level while the feature runs");
    assert_eq!(watch.newest(&health), None, "the feature latched off keeps no level read");
}

#[test]
#[cfg_attr(not(debug_assertions), ignore = "the counting allocator exists only in debug builds")]
fn the_read_and_its_lines_make_no_heap_allocation() {
    let mut memory = TestClient::new();
    let mut watch = memory.watch();
    let client = memory.client_address();
    let mut allocated = None;
    let ((), text) = logged("game-state-allocation", || {
        let guard = arm_alloc_guard().expect("arm the allocation guard");
        for id in [6, 6, 7] {
            memory.set_id(id);
            watch.on_frame();
        }
        memory.set_client(UNMAPPED);
        watch.on_frame();
        memory.set_client(0);
        watch.on_frame();
        memory.set_client(client);
        allocated = Some(guard.count());
    });
    assert_eq!(allocated, Some(0), "the game state read allocated");
    assert_eq!(game_states(&text).len(), 2, "the guarded frames logged both lines: {text}");
    assert!(text.contains(READ_FAILED), "the guarded frames logged the warning: {text}");
}

/// The install in the test exe, which holds no signature, under `health`: it installs nothing, and
/// its one log line is a warning that ends with ` error="<cause>"`.
fn assert_failed_install(name: &str, health: &Health, cause: &str) {
    let code = GameCode::copy();
    let (installed, text) = logged(name, || install(health, &code));
    assert!(installed.is_none(), "a game state read in the test exe");
    assert_eq!(health.state(Feature::GameState), FeatureState::Unavailable);
    let failures: Vec<&str> = text.lines().filter(|line| line.contains(NOT_INSTALLED)).collect();
    let [failure] = failures.as_slice() else { panic!("one failure line: {text}") };
    let expected_end = format!(" error=\"{cause}\"");
    assert!(failure.contains(" WARN  ") && failure.ends_with(&expected_end), "{failure}");
}

#[test]
fn a_signature_without_a_match_leaves_the_feature_off_and_logs_the_cause() {
    let health = Health::new();
    health.install_ok(Feature::PresentHook);
    let cause = "the g_game_client signature yields no address: the pattern has no match";
    assert_failed_install("game-state-install", &health, cause);
}

#[test]
fn no_running_present_hook_installs_nothing_and_logs_the_cause() {
    let cause =
        "the Present call-site hook, which runs the game state read each frame, is not running";
    assert_failed_install("game-state-no-present", &Health::new(), cause);
}
