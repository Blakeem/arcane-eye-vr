//! The view kind decision for each input alone and in combination, and its change line.

use game_bindings::game_client::StateId;
use ipc_layout::launch_token::Mode;
use ipc_layout::viewer_header::view_wish::ViewWish;

use super::{GameRead, ViewKind, decide, log_change};
use crate::logged;
use crate::vr_view::Rays;

/// The title screen's state id.
const TITLE: StateId = StateId::new(7);

/// The state id of a running level.
const LEVEL: StateId = StateId::new(18);

/// The state id of one step of a level load.
const LOADING: StateId = StateId::new(10);

/// The reads of `state_id` and `cinematic`, each present.
const fn read(state_id: StateId, cinematic: u8) -> GameRead {
    GameRead { state_id: Some(state_id), cinematic: Some(cinematic) }
}

/// The view kind in the mode `vr` with rewritten rays and the view wish world at the reads `game`.
fn in_vr(game: GameRead) -> ViewKind {
    decide(Mode::Vr, Rays::Rewritten, game, ViewWish::World)
}

#[test]
fn a_level_with_no_cinematic_in_the_mode_vr_with_rewritten_rays_is_a_world_frame() {
    assert_eq!(in_vr(read(LEVEL, 0)), ViewKind::World);
}

#[test]
fn the_view_wish_screen_makes_a_level_with_no_cinematic_a_screen_frame() {
    let kind = decide(Mode::Vr, Rays::Rewritten, read(LEVEL, 0), ViewWish::Screen);
    assert_eq!(kind, ViewKind::Screen);
}

#[test]
fn the_title_screen_a_load_and_a_cinematic_each_make_a_screen_frame() {
    let cases = [
        (read(TITLE, 0), "the title screen"),
        (read(LOADING, 0), "a level load"),
        (read(LEVEL, 1), "a dialogue in a level"),
        (read(TITLE, 1), "the title screen with the byte 1"),
        (read(LOADING, 1), "a level load with the byte 1"),
    ];
    for (game, what) in cases {
        assert_eq!(in_vr(game), ViewKind::Screen, "{what}");
    }
}

#[test]
fn a_failed_read_and_a_game_state_feature_that_is_not_installed_make_a_screen_frame() {
    let cases = [
        (GameRead { state_id: None, cinematic: Some(0) }, "no state id"),
        (GameRead { state_id: Some(LEVEL), cinematic: None }, "no cinematic byte"),
        (GameRead::default(), "neither read"),
    ];
    for (game, what) in cases {
        assert_eq!(in_vr(game), ViewKind::Screen, "{what}");
    }
}

#[test]
fn the_games_rays_and_the_mode_desktop_stereo_make_a_screen_frame_in_a_level() {
    let (level, world) = (read(LEVEL, 0), ViewWish::World);
    let game_rays = decide(Mode::Vr, Rays::GameBuilt, level, world);
    assert_eq!(game_rays, ViewKind::Screen, "the game's rays");
    for rays in [Rays::Rewritten, Rays::GameBuilt] {
        let kind = decide(Mode::DesktopStereo, rays, level, world);
        assert_eq!(kind, ViewKind::Screen, "desktop-stereo with {rays:?}");
    }
    let kind = decide(Mode::DesktopStereo, Rays::GameBuilt, read(TITLE, 1), ViewWish::Screen);
    assert_eq!(kind, ViewKind::Screen, "every input off");
}

#[test]
fn the_change_line_names_the_new_kind_the_wish_the_state_id_and_the_cinematic_byte() {
    let ((), text) = logged("view-kind-change", || {
        log_change(ViewKind::World, read(LEVEL, 0), ViewWish::World);
        log_change(ViewKind::Screen, GameRead::default(), ViewWish::World);
        log_change(ViewKind::Screen, read(LEVEL, 0), ViewWish::Screen);
    });
    let lines: Vec<&str> = text.lines().collect();
    let [world, screen, wished] = lines.as_slice() else {
        panic!("three change lines: {lines:#?}")
    };
    let level = " msg=\"view kind changed\" view=world wish=world state_id=18 cinematic=0";
    assert!(world.ends_with(level), "{world}");
    let no_reads = " msg=\"view kind changed\" view=screen wish=world state_id=-1 cinematic=-1";
    assert!(screen.ends_with(no_reads), "{screen}");
    let screen_wish = " msg=\"view kind changed\" view=screen wish=screen state_id=18 cinematic=0";
    assert!(wished.ends_with(screen_wish), "{wished}");
}
