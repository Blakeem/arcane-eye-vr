//! The lens in use of the launch mode `vr` at the Present call: a world frame's from the lens wish
//! the viewer wrote or from the dev flags, a screen frame's the game's, and the `vr view` line's
//! lens fields.

use std::time::{Duration, Instant};

use game_bindings::game_client::StateId;
use ipc_layout::launch_token::Mode;
use ipc_layout::viewer_header::STALE_AFTER;
use ipc_layout::viewer_header::lens_wish::LensWish;
use settings::dev::VrFlags;
use settings::dev::lens::{LensAspect, LensFovYDeg};
use settings::player::StereoSettings;
use viewer_client::header::{read_header, write_lens_wish};

use super::{call_times, frame, log_lines, timed_frame};
use crate::eye_camera::Placement;
use crate::frame::FrameState;
use crate::raw::test_support::{TestSwapchain, TestViewer, test_view_names};
use crate::view_camera::FrameCameras;
use crate::viewer::ViewerLink;
use crate::vr_view::{GameRead, Rays, VrView};

/// The reads of play in a level: the state id 18 and the cinematic byte 0.
const PLAY: GameRead = GameRead { state_id: Some(StateId::new(18)), cinematic: Some(0) };

/// The reads of the title screen: the state id 7.
const TITLE: GameRead = GameRead { state_id: Some(StateId::new(7)), cinematic: Some(0) };

/// The lens wish 100 degrees high with the aspect 1.
const WISH: LensWish = LensWish { fov_y_rad: 1.745_329_3, aspect: 1.0 };

/// A frame state of the test `test` in the mode `vr` whose dev flags' lens is 90 degrees high
/// with the aspect 1.5, after its first frame, and a viewing side that beats.
fn flagged_state(test: &str, swapchain: &TestSwapchain) -> (FrameState, TestViewer) {
    let lens_fov_y_deg = LensFovYDeg::try_from(90.0).expect("a field of view the flag takes");
    let lens_aspect = LensAspect::try_from(1.5).expect("an aspect the flag takes");
    let flags = VrFlags { lens_fov_y_deg, lens_aspect, ..VrFlags::default() };
    let link = ViewerLink::new(test_view_names(test));
    let mut state =
        FrameState { viewer: link, vr: VrView::new(Mode::Vr, flags), ..FrameState::default() };
    frame(&mut state, swapchain);
    let mut viewer = TestViewer::open(&test_view_names(test));
    viewer.beat();
    (state, viewer)
}

/// The value of the field `name` of `line`.
fn field(line: &str, name: &str) -> f32 {
    let start = format!(" {name}=");
    let value = line.split(&start).nth(1).and_then(|rest| rest.split(' ').next());
    value.and_then(|text| text.parse().ok()).unwrap_or_else(|| panic!("{name} in {line}"))
}

/// The `vr view` line of a perf window of `state` whose Present calls read `game`, and the
/// placement of the window's last call.
fn window_line(
    name: &str,
    state: &mut FrameState,
    viewer: &mut TestViewer,
    game: GameRead,
) -> (String, Placement) {
    let test = TestSwapchain::new(32, 32);
    let stereo = StereoSettings::default();
    let mut placement = None;
    let lines = log_lines(name, || {
        for now in call_times() {
            viewer.beat();
            placement = Some(state.hand_placement(stereo, Rays::Rewritten, game, now));
            timed_frame(state, &test, now, FrameCameras::default());
        }
    });
    let vr: Vec<&String> = lines.iter().filter(|line| line.contains("\"vr view\"")).collect();
    let [line] = vr.as_slice() else { panic!("one vr view line: {lines:#?}") };
    ((*line).clone(), placement.expect("a placement of the window"))
}

/// The lens `placement` gives a world frame's proxies, as its field of view and aspect.
fn placed_lens(placement: Placement) -> Option<(f32, f32)> {
    match placement {
        Placement::Pose { lens, .. } => lens.map(|lens| (lens.fov_y_rad(), lens.aspect())),
        Placement::Head { .. } => None,
    }
}

fn assert_lens(line: &str, fov_y_deg: f32, aspect: f32, from: &str) {
    let found = (field(line, "lens_fov_y_deg"), field(line, "lens_aspect"));
    let near = (found.0 - fov_y_deg).abs() < 1e-3 && (found.1 - aspect).abs() < 1e-6;
    assert!(near, "{found:?}, not ({fov_y_deg}, {aspect}): {line}");
    assert!(line.ends_with(&format!(" lens_from={from}")), "{line}");
}

#[test]
fn the_vr_view_line_holds_the_lens_of_the_wish_of_the_dev_flags_and_of_a_screen_frame() {
    let swapchain = TestSwapchain::new(32, 32);
    let (mut state, mut viewer) = flagged_state("vr-lens-line", &swapchain);
    let (flagged, flag_placement) = window_line("vr-lens-flag", &mut state, &mut viewer, PLAY);
    assert_lens(&flagged, 90.0, 1.5, "flag");
    let flag_lens = placed_lens(flag_placement).expect("the flags' lens in the placement");
    let quarter_turn = std::f32::consts::FRAC_PI_2;
    assert!((flag_lens.0 - quarter_turn).abs() < 1e-6 && (flag_lens.1 - 1.5).abs() < 1e-6);

    let names = test_view_names("vr-lens-line");
    let mut slot = None;
    let (view, _header) = read_header(&mut slot, &names.mapping).expect("the header");
    write_lens_wish(view, WISH).expect("write the lens wish");
    let (wished, wish_placement) = window_line("vr-lens-wish", &mut state, &mut viewer, PLAY);
    assert_lens(&wished, 100.0, 1.0, "wish");
    assert_eq!(placed_lens(wish_placement), Some((WISH.fov_y_rad, WISH.aspect)));

    let (screen, screen_placement) = window_line("vr-lens-screen", &mut state, &mut viewer, TITLE);
    assert!(screen.contains(" view=screen "), "{screen}");
    assert!(screen.ends_with(" lens_fov_y_deg=0.0 lens_aspect=0.0 lens_from=game"), "{screen}");
    assert_eq!(placed_lens(screen_placement), None, "a screen frame's placement");
}

#[test]
fn a_lens_wish_of_a_viewer_that_stopped_beating_is_reset_and_the_dev_flags_lens_returns() {
    let swapchain = TestSwapchain::new(32, 32);
    let (mut state, _viewer) = flagged_state("vr-lens-stale", &swapchain);
    let names = test_view_names("vr-lens-stale");
    let mut slot = None;
    let (view, _header) = read_header(&mut slot, &names.mapping).expect("the header");
    write_lens_wish(view, WISH).expect("write the lens wish");
    let stereo = StereoSettings::default();
    let now = Instant::now();
    let beating = state.hand_placement(stereo, Rays::Rewritten, PLAY, now);
    assert_eq!(placed_lens(beating), Some((WISH.fov_y_rad, WISH.aspect)), "a beating viewer");
    let stalled_at = now
        .checked_add(STALE_AFTER)
        .and_then(|limit| limit.checked_add(Duration::from_millis(100)))
        .expect("a test time past the heartbeat's limit");
    let stalled = state.hand_placement(stereo, Rays::Rewritten, PLAY, stalled_at);
    let flag_lens = placed_lens(stalled).map(|(fov, aspect)| (fov.to_degrees(), aspect));
    let flagged =
        |(fov, aspect): (f32, f32)| (fov - 90.0).abs() < 1e-3 && (aspect - 1.5).abs() < 1e-6;
    assert!(
        flag_lens.is_some_and(flagged),
        "the flags' lens once the viewer stopped: {flag_lens:?}"
    );
    let header = read_header(&mut slot, &names.mapping).expect("the header").1;
    assert_eq!(header.lens_wish(), None, "the stopped viewer's wish was reset");
}
