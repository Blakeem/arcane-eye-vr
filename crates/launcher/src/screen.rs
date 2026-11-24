//! The headset session: the game's published frames on a flat screen in the headset, the left half
//! of each frame to the left eye and the right half to the right eye, or in the immersive mode
//! around the player at the head pose each frame was rendered with.
//!
//! `picture.rs` reads the newest published frame from the viewer link, and `eyes.rs` draws its
//! halves into the swapchains of the `xr` crate's session. `immersive.rs` writes the head pose into
//! the viewer link and chooses between the projection layer and the screen quad for each frame, and
//! `layer.rs` builds that layer. `immersive/decide.rs` places each eye's view. In both views,
//! `share.rs` writes the screen quad's parallel share into the viewer link in each frame that
//! shows the game's frame on it, so the stereo depth follows the screen's width, and `depth.rs`
//! reads the core's depth map, through which the pointer traces its ray on the screen quad and in
//! the projection layer. `quad_depth.rs` runs those two steps in each frame. `controls.rs` reads
//! the device profiles and logs what the controllers do in each frame. `menu.rs` shows the menu
//! that sets the screen's distance and width, `pointer.rs` sends the game the mouse input of the
//! controller that points at the game's picture and the turn key input of the pointing
//! controller's stick, and `geometry.rs` holds the vector math of the quads, the rays and the eye
//! views. The menu and the pointer work in every frame. `models.rs` draws every tracked controller
//! model into one projection layer, which the compositor blends over the frame's layer and under
//! the panels. `image.rs` finds the draw target of each swapchain image that the eyes, the panels and
//! the models acquire. The session runs until Steam reports that the game ended, the runtime ends
//! the session or the game's frames are of a kind it cannot show. The game window keeps the
//! keyboard, since the session has no window, and the pointer's input reaches the game only while
//! the game window is in the foreground.
//!
//! A game that the launch app starts starts in the first pass of the frame loop after the session
//! runs (`crate::start`). Until the game reports its goal status, the session shows none of its
//! frames, and Steam's report that it does not run ends no screen. `game.rs` chooses a running
//! game at the start and watches Steam's report. `status.rs` shows the status panel until the
//! game's first frame shows, and `panel_image.rs` draws the pictures of the status panel and of
//! the menu into their panels. `launcher.rs` shows the Larian launcher's window on the screen in
//! place of the status panel while it exists during the start wait, and the pointer clicks it.

mod controls;
mod depth;
mod eyes;
pub(crate) mod failure;
pub(crate) mod game;
mod geometry;
mod image;
mod immersive;
mod launcher;
mod layer;
mod menu;
mod models;
mod panel_image;
mod picture;
mod pointer;
mod quad_depth;
mod share;
mod status;

use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use diag::error_chain;
use diag::once;
use diag::rate::Once;
use gfx::region::RegionDraw;
use ipc_layout::viewer_header::AdapterLuid;
use ipc_layout::viewer_header::pose::FrameTag;
use launch::checks;
use launch::token::{self, TokenMapping};
use settings::file::SETTINGS_FILE;
use settings::player::Settings;
use tracing::{Level, info, warn};
use viewer_client::header::Names;
use viewer_client::{Device, heartbeat};
use xr::error::XrError;
use xr::panel::Panel;
use xr::runtime::Runtime;
use xr::session::{Lifecycle, Session, Shown};
use xr::swapchain::Swapchains;

use self::controls::Controllers;
use self::eyes::{Eyes, EyesError};
use self::failure::ScreenFailure;
use self::game::GameWatch;
use self::immersive::Immersive;
use self::launcher::{LauncherScreen, Wait};
use self::layer::Layer;
use self::menu::{PlayerFile, ScreenMenu};
use self::models::Models;
use self::panel_image::Gpu;
use self::picture::{Link, Unshowable};
use self::pointer::{PanelAim, Pointer, Seen};
use self::quad_depth::QuadDepth;
use self::status::StatusPanel;
use crate::start::{self, Start, StartError};

/// The wait between two event polls while the session is not begun.
const IDLE_WAIT: Duration = Duration::from_millis(50);

/// How the session shows the game's frames.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum View {
    /// Every frame on the flat screen.
    Screen,
    /// Each frame with a kept head pose around the player, every other frame on the flat screen.
    Immersive,
}

/// The player file's settings in the root folder `root`, and whether the file gave them. A file
/// that cannot be read or is rejected gives the defaults and a warning, as it does in the core.
fn settings(root: &Path) -> (Settings, PlayerFile) {
    let (player, file) = match settings::file::read_settings(root) {
        Ok(parsed) => {
            for key in &parsed.unknown_keys {
                warn!(file = SETTINGS_FILE, key = %key, "unknown settings key ignored");
            }
            (parsed.settings, PlayerFile::Read)
        }
        Err(error) => {
            let error = error_chain::chain(&error);
            warn!(error = %error, "settings file gave no values, so its defaults apply");
            (Settings::default(), PlayerFile::Unread)
        }
    };
    let screen = player.screen;
    let (distance_m, width_m) = (screen.distance_m.meters(), screen.width_m.meters());
    info!(distance_m, width_m, "screen settings loaded");
    (player, file)
}

/// Why the headset session's run stopped before the game or the session ended.
#[derive(Debug, thiserror::Error)]
pub(crate) enum RunFailure {
    #[error(transparent)]
    Screen(#[from] ScreenFailure),
    #[error(transparent)]
    Start(#[from] StartError),
}

/// Runs `start` from the session's frame loop and shows the game's frames in the headset as `view`
/// says until the game ends or the runtime ends the session. `manifest` is the active runtime's
/// manifest, and `root` the root folder that holds the player file. Before it returns, the screen
/// menu's unsaved changes are written to a player file that gave its values at the start.
///
/// # Errors
///
/// Returns the [`ScreenFailure`] of a step before the session runs and of a session that failed,
/// and the [`StartError`] of the game's start.
pub(crate) fn run(
    manifest: &Path,
    root: &Path,
    view: View,
    start: Start<TokenMapping>,
) -> Result<(), RunFailure> {
    let mut screen = open(manifest, root, view, start)?;
    let shown = screen.show_until_end(&mut GameWatch::new(Instant::now()));
    screen.pointer.finish();
    screen.menu.finish();
    screen.launcher.finish();
    shown
}

/// Reads the player settings, starts the heartbeat thread and opens the session.
fn open(
    manifest: &Path,
    root: &Path,
    view: View,
    start: Start<TokenMapping>,
) -> Result<Screen, ScreenFailure> {
    let (player, file) = settings(root);
    let names = Names::default();
    if let Err(error) = heartbeat::spawn(names.mapping.clone()) {
        return Err(ScreenFailure::Heartbeat(error));
    }
    let runtime = Runtime::load(manifest)?;
    let headset = runtime.connect(controls::profiles())?;
    let luid = headset.adapter()?.luid();
    let device = match Device::create(AdapterLuid { low: luid.LowPart, high: luid.HighPart }) {
        Ok(device) => device,
        Err(error) => return Err(ScreenFailure::Device(error)),
    };
    let draw = RegionDraw::create(device.d3d())?;
    let session = Session::open(&headset, device.d3d())?;
    let immersive = match view {
        View::Screen => None,
        View::Immersive => Some(Immersive::new()),
    };
    let status_panel = if start.is_done() { StatusPanel::closed() } else { StatusPanel::new() };
    Ok(Screen {
        eyes: None,
        models: Models::new(),
        immersive,
        quad_depth: QuadDepth::new(player.stereo.ipd_mm.meters()),
        status_panel,
        launcher: LauncherScreen::new(),
        menu: ScreenMenu::new(player.screen, file, root),
        pointer: Pointer::new(),
        link: Link::new(names),
        session,
        draw,
        device,
        controllers: Controllers::new(),
        first_shown: Once::new(),
        start,
    })
}

/// Why a session frame failed.
#[derive(Debug, thiserror::Error)]
pub(crate) enum FrameError {
    #[error(transparent)]
    Xr(#[from] XrError),
    #[error(transparent)]
    Eyes(#[from] EyesError),
    #[error(transparent)]
    Unshowable(#[from] Unshowable),
}

impl FrameError {
    /// Whether the runtime lost the session, which ends the screen as the session's end does.
    fn is_loss(&self) -> bool {
        match self {
            Self::Xr(error) | Self::Eyes(EyesError::Xr(error)) => error.is_loss(),
            Self::Eyes(EyesError::Draw(_) | EyesError::NoImage { .. }) | Self::Unshowable(_) => {
                false
            }
        }
    }
}

/// Everything the screen opened. Fields drop in declaration order, so the swapchains and the
/// panels go before the session and the device goes last.
struct Screen {
    eyes: Option<Eyes>,
    models: Models,
    /// `None` in the view [`View::Screen`].
    immersive: Option<Immersive>,
    quad_depth: QuadDepth,
    status_panel: StatusPanel,
    launcher: LauncherScreen,
    menu: ScreenMenu,
    pointer: Pointer,
    link: Link,
    session: Session,
    draw: RegionDraw,
    device: Device,
    controllers: Controllers,
    first_shown: Once,
    /// Holds the launch token until the goal status, so the session's end drops it.
    start: Start<TokenMapping>,
}

impl Screen {
    fn show_until_end(&mut self, game: &mut GameWatch) -> Result<(), RunFailure> {
        loop {
            if game.ended(&self.start, Instant::now(), checks::game_running) {
                info!("Steam reports that the game ended, so the screen ends");
                return Ok(());
            }
            let (lifecycle, frame) = match self.session.poll() {
                Ok(Lifecycle::Ended) => {
                    info!("the runtime ended the session, so the screen ends");
                    return Ok(());
                }
                Ok(Lifecycle::Idle) => {
                    // thread::sleep is banned for game threads. Parking waits the same, and an
                    // early wake only polls again.
                    thread::park_timeout(IDLE_WAIT);
                    (Lifecycle::Idle, Ok(()))
                }
                Ok(Lifecycle::Running) => (Lifecycle::Running, self.frame()),
                Err(error) => return end_or_fail(FrameError::Xr(error)),
            };
            if let Err(error) = frame {
                return end_or_fail(error);
            }
            self.start.pass(Instant::now(), lifecycle, start::begin, token::read)?;
        }
    }

    /// Waits for the session's next frame, reads the controllers, writes the head pose in the view
    /// [`View::Immersive`], draws the newest published frame into the swapchains once the start is
    /// done and the runtime shows the frame, chooses its layer, updates the menu, writes the screen
    /// quad's parallel share while the quad shows a game frame, reads the depth map, shows the
    /// newest copy of the launcher window during the start wait, sends the pointer's input to the
    /// game or the launcher window, draws the menu's quads, the laser and the controller models,
    /// and ends the frame with the frame's layer, then the controller models' layer, then the
    /// launcher's picture or else the status panel until a game frame showed, then the menu's
    /// panels.
    fn frame(&mut self) -> Result<(), FrameError> {
        let timing = self.session.wait_frame()?;
        let controls = self.controllers.read(&self.session, timing)?;
        let now = Instant::now();
        let Self {
            eyes,
            models,
            immersive,
            quad_depth,
            status_panel,
            launcher,
            menu,
            pointer,
            link,
            session,
            draw,
            device,
            first_shown,
            start,
            ..
        } = self;
        // The view Immersive reads the head in every frame for its write, and the share write of
        // a screen quad frame takes that read, so the session locates the head once per frame.
        let head = immersive.as_ref().map(|_| session.locate_head(timing));
        if let Some((immersive, located)) = immersive.as_mut().zip(head) {
            immersive.write_head(located, link)?;
        }
        // A header left by an earlier game holds a texture no process shares, so the link reads
        // frames only once this start's game reported its goal status.
        let game = timing.should_render && start.is_done();
        let drawn = if game { show(eyes, link, session, draw, device)? } else { None };
        let choice = drawn.as_ref().zip(immersive.as_mut()).map(|(drawn, on)| on.choose(drawn.tag));
        let mut menu_shown = menu.update(&controls, now);
        let frame_layer = drawn.and_then(|drawn| Layer::of(drawn.frame, choice, menu.screen()));
        let shown = drawn.zip(frame_layer).map(|(drawn, on)| on.shown(drawn.swapchains));
        let game_quad = frame_layer.and_then(Layer::screen_quad);
        let locate = || head.unwrap_or_else(|| session.locate_head(timing));
        let depth = quad_depth.frame(game_quad, locate, link, now)?;
        let gpu = Gpu { session, draw, context: device.context() };
        let wait = if start.is_done() { Wait::Done } else { Wait::Running };
        launcher.update(now, wait, menu.screen(), &gpu);
        let target = launcher::target(wait, launcher.shown(), frame_layer);
        let panel = if menu_shown.points_at_panel() { PanelAim::Met } else { PanelAim::Missed };
        let seen = Seen { menu: menu.hand(), panel, target, depth };
        if let Some(laser) = pointer.update(&controls, seen, now) {
            menu_shown.laser = Some(laser);
        }
        menu.draw(menu_shown, &gpu);
        if let Some(Shown::Screen { swapchains, .. } | Shown::Projection { swapchains, .. }) = shown
        {
            let (width, height) = swapchains.size();
            once!(*first_shown, Level::INFO, width, height, "first frame shown in the headset");
            status_panel.close();
        }
        let start_quad = launcher.panel().or_else(|| status_panel.panel(|| status::make(&gpu)));
        let panels: Vec<&Panel> = start_quad.into_iter().chain(menu.panels()).collect();
        let model_layer = models.frame(session, device, timing)?;
        session.end_frame(timing, shown, model_layer, &panels)?;
        if let Some(immersive) = immersive.as_mut() {
            immersive.end_frame();
        }
        Ok(())
    }
}

/// Ends the screen with no failure when the runtime lost the session, and with `error` otherwise.
fn end_or_fail(error: FrameError) -> Result<(), RunFailure> {
    if error.is_loss() {
        info!(error = %error, "the runtime lost the session, so the screen ends");
        return Ok(());
    }
    Err(ScreenFailure::from(error).into())
}

/// The swapchains that hold each eye's half of a published frame, the entire frame's size and the
/// frame's tag.
#[derive(Clone, Copy)]
struct Drawn<'eyes> {
    swapchains: &'eyes Swapchains,
    frame: (u32, u32),
    tag: FrameTag,
}

/// Draws each eye's half of the newest published frame into its swapchain, or returns `None`
/// while no frame was published.
fn show<'eyes>(
    eyes: &'eyes mut Option<Eyes>,
    link: &mut Link,
    session: &Session,
    draw: &RegionDraw,
    device: &Device,
) -> Result<Option<Drawn<'eyes>>, FrameError> {
    let Some(picture) = link.refresh(device, draw)? else {
        return Ok(None);
    };
    let swapchains = eyes::show(eyes, session, draw, device.context(), picture)?;
    let (frame, tag) = (picture.size(), picture.tag());
    Ok(swapchains.map(|swapchains| Drawn { swapchains, frame, tag }))
}

#[cfg(test)]
mod tests;
