//! The screen menu across frames of controller input and its writes of the player file. `pixels.rs`
//! holds the draw of its pixels into a panel image.

mod pixels;

use std::cell::Cell;
use std::f32::consts::FRAC_1_SQRT_2;
use std::fs;
use std::time::{Duration, Instant};

use device_profile::profile::Hand;
use openxr::{Posef, Quaternionf, Vector3f};
use overlay::layout::{self, PANEL_HEIGHT, PANEL_WIDTH};
use overlay::menu::Button;
use settings::file;
use settings::player::{ScreenDistanceM, ScreenSettings, ScreenWidthM};
use xr::input::{ButtonState, HandInput, Input};

use super::model::{Event, Model, Place, Shown, toggled};
use super::place::PANEL_SIZE;
use super::save::{PlayerFile, Saver, Saving};
use super::{MenuError, PanelImageError, ScreenMenu, rows};
use crate::screen::controls::{ControllerWatch, Frame};
use crate::screen::geometry::rotate;
use crate::screen::geometry::test_support::{assert_near, vector};

const fn pose(position: Vector3f) -> Posef {
    Posef { orientation: Quaternionf::IDENTITY, position }
}

/// The screen values `distance_m` and `width_m`, which lie in range.
pub(super) fn screen(distance_m: f32, width_m: f32) -> ScreenSettings {
    ScreenSettings {
        distance_m: ScreenDistanceM::try_from(distance_m).expect("a distance in range"),
        width_m: ScreenWidthM::try_from(width_m).expect("a width in range"),
    }
}

/// The left grip of a thumb-up hand, which puts the panel upright at (0, 0, -0.5) facing +Z.
const GRIP: Posef = Posef {
    orientation: Quaternionf { x: FRAC_1_SQRT_2, y: 0.0, z: 0.0, w: FRAC_1_SQRT_2 },
    position: vector(0.0, -0.15, -0.5),
};

/// A menu, the controllers' watch and the time of the next frame, with the menu on the left hand
/// and the right hand pointing.
struct Run {
    model: Model,
    watch: ControllerWatch,
    now: Instant,
    events: Vec<Event>,
}

/// One frame's hands: the left hand's menu and grip, and the right hand's menu, aim and select.
#[derive(Clone, Copy, Default)]
struct Hands {
    menu: ButtonState,
    grip: Option<Posef>,
    right_menu: ButtonState,
    aim: Option<Vector3f>,
    select: f32,
}

impl Run {
    fn new(values: ScreenSettings) -> Self {
        let now = Instant::now();
        Self {
            model: Model::new(values),
            watch: ControllerWatch::default(),
            now,
            events: Vec::new(),
        }
    }

    /// Runs one frame of `hands` at the run's time, then moves the time on by `step`.
    fn frame(&mut self, hands: Hands, step: Duration) -> Shown {
        let frame = controls(&mut self.watch, hands);
        let events = &mut self.events;
        let shown = self.model.update(&frame, self.now, &mut |event| events.push(event));
        let shown = shown.expect("two rows");
        self.now = self.now.checked_add(step).expect("a later time");
        shown
    }

    /// Presses and releases the left hand's menu button with the left grip held at [`GRIP`].
    fn press_menu(&mut self) {
        let grip = Some(GRIP);
        self.frame(Hands { menu: ButtonState::Pressed, grip, ..Hands::default() }, Duration::ZERO);
        self.frame(Hands { grip, ..Hands::default() }, Duration::ZERO);
    }

    fn steps(&self) -> usize {
        self.events.iter().filter(|event| matches!(event, Event::Stepped(_))).count()
    }
}

/// The controllers' frame of `hands`, read through `watch`.
fn controls(watch: &mut ControllerWatch, hands: Hands) -> Frame {
    let input = Input {
        left: HandInput { menu: hands.menu, grip: hands.grip, ..HandInput::default() },
        right: HandInput {
            menu: hands.right_menu,
            aim: hands.aim.map(pose),
            select: hands.select,
            ..HandInput::default()
        },
        head: None,
    };
    let (left, right) = watch.update(&input, |_| {});
    Frame { input, left, right }
}

/// The right aim position in front of the center of the `button` of the distance row, so that its
/// ray along -Z meets the panel there.
fn aim_at(values: ScreenSettings, button: Button) -> Vector3f {
    let laid = layout::layout(&rows::menu(values).expect("two rows"));
    let rect = laid.rows.first().expect("the distance row").button(button).rect;
    let center_x = f32::from(rect.x) + f32::from(rect.width) / 2.0;
    let center_y = f32::from(rect.y) + f32::from(rect.height) / 2.0;
    let x = (center_x / f32::from(PANEL_WIDTH) - 0.5) * PANEL_SIZE.width;
    let y = (0.5 - center_y / f32::from(PANEL_HEIGHT)) * PANEL_SIZE.height;
    vector(x, y, 0.0)
}

#[test]
fn a_menu_press_opens_closes_or_moves_the_menu() {
    assert_eq!(toggled(Place::Closed, Hand::Left), Place::On(Hand::Left));
    assert_eq!(toggled(Place::On(Hand::Left), Hand::Left), Place::Closed);
    assert_eq!(toggled(Place::On(Hand::Right), Hand::Left), Place::On(Hand::Left));
}

#[test]
fn each_open_move_and_close_is_one_event_with_its_hand() {
    let mut run = Run::new(screen(2.5, 3.0));
    run.press_menu();
    let right_press = Hands { right_menu: ButtonState::Pressed, ..Hands::default() };
    run.frame(right_press, Duration::ZERO);
    run.frame(Hands::default(), Duration::ZERO);
    run.press_menu();
    run.press_menu();
    assert_eq!(
        run.events,
        [
            Event::Opened(Hand::Left),
            Event::Moved(Hand::Right),
            Event::Moved(Hand::Left),
            Event::Closed(Hand::Left),
        ]
    );
}

#[test]
fn a_menu_press_opens_the_menu_in_a_projection_frame_since_the_menu_takes_no_layer() {
    // The menu's update takes the controllers and the time only, so the frame's layer, the screen
    // quad or the projection picture, cannot rest it.
    let mut run = Run::new(screen(2.5, 3.0));
    let pressed = Hands { menu: ButtonState::Pressed, grip: Some(GRIP), ..Hands::default() };
    let shown = run.frame(pressed, Duration::ZERO);
    assert!(shown.panel.is_some(), "the press opens the menu's panel");
    assert_eq!(run.events, [Event::Opened(Hand::Left)]);
}

#[test]
fn a_press_held_for_one_second_over_a_button_gives_seven_steps() {
    let values = screen(2.5, 3.0);
    let mut run = Run::new(values);
    run.press_menu();
    let aim = Some(aim_at(values, Button::Plus));
    let held = Hands { grip: Some(GRIP), aim, select: 1.0, ..Hands::default() };
    for _ in 0..20 {
        run.frame(held, Duration::from_millis(50));
    }
    assert_eq!(run.steps(), 7, "one at the press, one at 0.4 s and five to 0.9 s");
    assert_eq!(run.model.screen(), screen(3.2, 3.0));
}

#[test]
fn a_press_that_leaves_its_button_stops_repeating_and_one_that_starts_off_it_gives_no_step() {
    let values = screen(2.5, 3.0);
    let mut run = Run::new(values);
    run.press_menu();
    let on = Hands {
        grip: Some(GRIP),
        aim: Some(aim_at(values, Button::Minus)),
        select: 1.0,
        ..Hands::default()
    };
    let off = Hands { aim: Some(vector(0.0, 0.0, 0.0)), ..on };
    run.frame(on, Duration::from_millis(500));
    run.frame(off, Duration::from_millis(100));
    run.frame(on, Duration::from_millis(500));
    assert_eq!(run.steps(), 1, "only the press over the button steps");
    run.frame(Hands { select: 0.0, ..on }, Duration::ZERO);
    run.frame(off, Duration::ZERO);
    run.frame(on, Duration::from_millis(500));
    assert_eq!(run.steps(), 1, "a press that starts off the buttons");
    assert_eq!(run.model.screen(), screen(2.4, 3.0));
}

#[test]
fn the_panel_shows_from_the_first_grip_and_keeps_its_pose_without_one() {
    let mut run = Run::new(screen(2.5, 3.0));
    let open =
        Hands { menu: ButtonState::Pressed, aim: Some(vector(5.0, 0.0, 0.0)), ..Hands::default() };
    let shown = run.frame(open, Duration::ZERO);
    assert_eq!(shown.panel, None, "no grip pose yet");
    let laser = shown.laser.expect("the pointing hand's laser");
    assert_eq!(laser.size.height, 1.0, "a laser that meets no panel");
    let gripped = run.frame(Hands { grip: Some(GRIP), ..Hands::default() }, Duration::ZERO);
    let placed = gripped.panel.expect("the panel").pose;
    assert_near(placed.position, vector(0.0, 0.0, -0.5));
    let kept = run.frame(Hands::default(), Duration::ZERO);
    assert_eq!(kept.panel.map(|panel| panel.pose), Some(placed));
    assert_eq!(kept.laser, None, "no aim pose");
}

#[test]
fn the_open_panel_turns_when_only_the_grip_s_orientation_changes() {
    let mut run = Run::new(screen(2.5, 3.0));
    run.press_menu();
    let before = run.frame(Hands { grip: Some(GRIP), ..Hands::default() }, Duration::ZERO).panel;
    let level = Posef { orientation: Quaternionf::IDENTITY, ..GRIP };
    let after = run.frame(Hands { grip: Some(level), ..Hands::default() }, Duration::ZERO).panel;
    let [before, after] = [before, after].map(|panel| panel.expect("the panel").pose);
    assert_near(rotate(before.orientation, vector(0.0, 0.0, 1.0)), vector(0.0, 0.0, 1.0));
    assert_near(rotate(after.orientation, vector(0.0, 0.0, 1.0)), vector(0.0, 1.0, 0.0));
}

#[test]
fn the_laser_ends_on_the_panel_and_the_point_lies_over_the_button() {
    let values = screen(2.5, 3.0);
    let mut run = Run::new(values);
    run.press_menu();
    let aim = Some(aim_at(values, Button::Plus));
    let shown = run.frame(Hands { grip: Some(GRIP), aim, ..Hands::default() }, Duration::ZERO);
    let laser = shown.laser.expect("the laser");
    assert!((laser.size.height - 0.5).abs() < 1e-5, "from z 0 to the panel at z -0.5");
    let point = shown.panel.and_then(|panel| panel.point).expect("a point on the panel");
    let laid = layout::layout(&rows::menu(values).expect("two rows"));
    let hit = layout::hit(&laid, point).expect("a button");
    assert_eq!((hit.row, hit.button), (0, Button::Plus));
}

#[test]
fn a_closed_menu_after_two_steps_writes_once_and_one_with_no_step_writes_nothing() {
    let values = screen(2.5, 3.0);
    let mut run = Run::new(values);
    let mut saver = Saver::new(values, PlayerFile::Read);
    let writes = Cell::new(0_u32);
    let save = |held: &mut Saver, current| {
        held.save(current, |_| {
            writes.set(writes.get().saturating_add(1));
            Ok(())
        })
    };
    run.press_menu();
    let on =
        Hands { grip: Some(GRIP), aim: Some(aim_at(values, Button::Plus)), ..Hands::default() };
    for select in [1.0, 0.0, 1.0, 0.0] {
        run.frame(Hands { select, ..on }, Duration::from_millis(50));
    }
    run.press_menu();
    assert_eq!(run.events.last(), Some(&Event::Closed(Hand::Left)));
    assert!(matches!(save(&mut saver, run.model.screen()), Saving::Written(_)));
    assert_eq!(writes.get(), 1);
    run.press_menu();
    run.press_menu();
    assert!(matches!(save(&mut saver, run.model.screen()), Saving::Unchanged));
    assert_eq!(writes.get(), 1, "a closed menu with no step");
}

/// Takes frames through `menu`, which shows `values`, that open the menu, take one step and close
/// it with a press.
fn step_and_close(menu: &mut ScreenMenu, values: ScreenSettings) {
    let (grip, aim) = (Some(GRIP), Some(aim_at(values, Button::Plus)));
    let menu_press = Hands { menu: ButtonState::Pressed, grip, ..Hands::default() };
    let mut watch = ControllerWatch::default();
    let now = Instant::now();
    for hands in [
        menu_press,
        Hands { grip, aim, select: 1.0, ..Hands::default() },
        Hands { grip, aim, ..Hands::default() },
        menu_press,
    ] {
        menu.take(&controls(&mut watch, hands), now).expect("two rows");
    }
}

#[test]
fn a_close_after_a_step_writes_the_player_file() {
    let dir = test_dir("menu-close");
    let mut menu = ScreenMenu::new(screen(2.5, 3.0), PlayerFile::Read, dir.path());
    step_and_close(&mut menu, screen(2.5, 3.0));
    let read = file::read_settings(dir.path()).expect("read the player file");
    assert_eq!(read.settings.screen, screen(2.6, 3.0));
}

#[test]
fn a_write_that_failed_at_the_close_runs_again_at_the_session_end() {
    let dir = test_dir("menu-finish-retry");
    let blocker = dir.path().join("settings.toml.new");
    fs::create_dir(&blocker).expect("a folder in the new file's place");
    let mut menu = ScreenMenu::new(screen(2.5, 3.0), PlayerFile::Read, dir.path());
    step_and_close(&mut menu, screen(2.5, 3.0));
    assert!(!dir.path().join(file::SETTINGS_FILE).exists(), "the write at the close failed");
    fs::remove_dir(&blocker).expect("remove the folder");
    menu.finish();
    let read = file::read_settings(dir.path()).expect("read the player file");
    assert_eq!(read.settings.screen, screen(2.6, 3.0));
}

#[test]
fn a_change_never_reaches_a_player_file_that_gave_no_values() {
    let dir = test_dir("menu-close-unread");
    let path = dir.path().join(file::SETTINGS_FILE);
    let text = "[stereo]\nipd_mm = 90.0\n\n[screen]\ndistance_m = 5.0\nwidth_m = 6.0\n";
    fs::write(&path, text).expect("write a file with an IPD out of range");
    let mut menu = ScreenMenu::new(screen(2.5, 3.0), PlayerFile::Unread, dir.path());
    step_and_close(&mut menu, screen(2.5, 3.0));
    menu.finish();
    assert_eq!(menu.screen(), screen(2.6, 3.0), "the step changed the screen");
    assert_eq!(fs::read_to_string(&path).expect("read the player file"), text);
}

/// A menu over the player file of `dir` whose model is open on the left hand after one plus step
/// of the distance from 2.5 meters.
fn stepped_menu(dir: &test_support::TestDir) -> ScreenMenu {
    let values = screen(2.5, 3.0);
    let mut run = Run::new(values);
    run.press_menu();
    let aim = Some(aim_at(values, Button::Plus));
    run.frame(Hands { grip: Some(GRIP), aim, select: 1.0, ..Hands::default() }, Duration::ZERO);
    assert_eq!(run.model.screen(), screen(2.6, 3.0));
    let mut menu = ScreenMenu::new(values, PlayerFile::Read, dir.path());
    menu.model = run.model;
    menu
}

fn test_dir(name: &str) -> test_support::TestDir {
    test_support::TestDir::in_temp("arcane-eye-launcher", name).expect("create the test folder")
}

#[test]
fn a_session_that_ends_with_the_menu_open_writes_its_change_once() {
    let dir = test_dir("menu-finish");
    let mut menu = stepped_menu(&dir);
    menu.finish();
    let read = file::read_settings(dir.path()).expect("read the player file");
    assert_eq!(read.settings.screen, screen(2.6, 3.0));
    assert!(matches!(menu.saver.save(menu.screen(), |_| Ok(())), Saving::Unchanged));
}

#[test]
fn a_session_that_ends_with_the_menu_closed_writes_nothing() {
    let dir = test_dir("menu-finish-closed");
    let mut menu = ScreenMenu::new(screen(2.5, 3.0), PlayerFile::Read, dir.path());
    menu.finish();
    assert!(!dir.path().join(file::SETTINGS_FILE).exists(), "no player file was written");
}

#[test]
fn a_failed_menu_closes_writes_its_change_and_shows_no_panel() {
    let dir = test_dir("menu-failure");
    let mut menu = stepped_menu(&dir);
    menu.fail(&MenuError::Image(PanelImageError::NoImage { index: 7 }));
    assert!(menu.drawing.is_none(), "the menu is gone for the session");
    assert_eq!(menu.model.close(), None, "the menu was closed");
    assert!(menu.panels().is_empty());
    let read = file::read_settings(dir.path()).expect("read the player file");
    assert_eq!(read.settings.screen, screen(2.6, 3.0));
}
