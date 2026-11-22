//! The headset session: the game's published frames on a flat screen in the headset, the left half
//! of each frame to the left eye and the right half to the right eye, or in the immersive mode
//! around the player at the head pose each frame was rendered with.
//!
//! `picture.rs` reads the newest published frame from the viewer link, and `eyes.rs` draws its
//! halves into the swapchains of the `xr` crate's session. `immersive.rs` writes the head pose into
//! the viewer link and chooses between the projection layer and the screen quad for each frame, and
//! `layer.rs` builds that layer. `immersive/decide.rs` places each eye's view. `controls.rs` reads
//! the device profiles and logs what the controllers do in each frame. `menu.rs` shows the menu
//! that sets the screen's distance and width, `pointer.rs` sends the game the mouse input of the
//! controller that points at the screen and the turn key input of the pointing controller's stick,
//! and `geometry.rs` holds the vector math of the quads and rays. The menu and the pointer rest while a projection layer shows. The session
//! runs until Steam reports that the game ended, the runtime ends the session or the game's frames
//! are of a kind it cannot show. The game window keeps the keyboard, since the session has no
//! window, and the pointer's input reaches the game only while the game window is in the
//! foreground.

mod controls;
mod eyes;
pub(crate) mod failure;
mod geometry;
mod immersive;
mod layer;
mod menu;
mod picture;
mod pointer;

use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use diag::error_chain;
use diag::once;
use diag::rate::Once;
use gfx::region::RegionDraw;
use ipc_layout::viewer_header::pose::FrameTag;
use ipc_layout::viewer_header::{self, AdapterLuid};
use launch::checks;
use launch::registry::RegistryError;
use settings::file::SETTINGS_FILE;
use settings::player::ScreenSettings;
use tracing::{Level, info, warn};
use viewer_client::header::Names;
use viewer_client::{Device, HeaderView, HeaderViewError, heartbeat};
use xr::error::XrError;
use xr::runtime::Runtime;
use xr::session::{Lifecycle, Session, Shown};
use xr::swapchain::Swapchains;

use self::controls::Controllers;
use self::eyes::{Eyes, EyesError};
use self::failure::ScreenFailure;
use self::immersive::Immersive;
use self::layer::Layer;
use self::menu::{Gpu, PlayerFile, ScreenMenu};
use self::picture::{Link, Unshowable};
use self::pointer::{PanelAim, Pointer, Sight};

/// The wait between two event polls while the session is not begun.
const IDLE_WAIT: Duration = Duration::from_millis(50);

/// The time between two reads of Steam's report that the game runs.
const GAME_CHECK_INTERVAL: Duration = Duration::from_secs(1);

/// How the session shows the game's frames.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum View {
    /// Every frame on the flat screen.
    Screen,
    /// Each frame with a kept head pose around the player, every other frame on the flat screen.
    Immersive,
}

/// The game the headset session shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GameChoice {
    /// A game with the mod runs, so the session shows it and no game starts.
    Running,
    /// The launch app starts the game, then the session shows it.
    Start,
}

/// Steam's report of whether the game runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SteamReport {
    Running,
    NotRunning,
}

/// Whether the viewer header's mapping exists.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Header {
    Mapped,
    Absent,
}

/// The game the headset session shows, from Steam's report and the viewer header's mapping. A
/// failed read of Steam's report leaves the choice to the mapping.
///
/// # Errors
///
/// Returns the [`HeaderViewError`] of a mapping that exists and cannot be opened.
pub(crate) fn find_game() -> Result<GameChoice, HeaderViewError> {
    let header = match HeaderView::open(viewer_header::MAPPING_NAME)? {
        Some(_) => Header::Mapped,
        None => Header::Absent,
    };
    let steam = match checks::game_running() {
        Ok(true) => SteamReport::Running,
        Ok(false) => SteamReport::NotRunning,
        Err(error) => {
            warn!(
                error = %error,
                "could not read whether the game runs, so the viewer header decides"
            );
            SteamReport::Running
        }
    };
    Ok(choose_game(steam, header))
}

/// A running game only when Steam reports it and the header is mapped. The core creates the header
/// mapping at start, but a reader such as `viewer.exe` keeps it open after the game quits, so the
/// mapping alone does not show a running game.
const fn choose_game(steam: SteamReport, header: Header) -> GameChoice {
    match (steam, header) {
        (SteamReport::Running, Header::Mapped) => GameChoice::Running,
        (SteamReport::NotRunning, _) | (_, Header::Absent) => GameChoice::Start,
    }
}

/// The table `[screen]` of the player file in the root folder `root`, and whether the file gave it.
/// A file that cannot be read or is rejected gives the defaults and a warning, as it does in the
/// core.
fn settings(root: &Path) -> (ScreenSettings, PlayerFile) {
    let (screen, file) = match settings::file::read_settings(root) {
        Ok(parsed) => {
            for key in &parsed.unknown_keys {
                warn!(file = SETTINGS_FILE, key = %key, "unknown settings key ignored");
            }
            (parsed.settings.screen, PlayerFile::Read)
        }
        Err(error) => {
            let error = error_chain::chain(&error);
            warn!(error = %error, "settings file gave no values, so its defaults apply");
            (ScreenSettings::default(), PlayerFile::Unread)
        }
    };
    let (distance_m, width_m) = (screen.distance_m.meters(), screen.width_m.meters());
    info!(distance_m, width_m, "screen settings loaded");
    (screen, file)
}

/// Shows the game's frames in the headset as `view` says until the game ends or the runtime ends
/// the session. `manifest` is the active runtime's manifest, and `root` the root folder that holds
/// the player file. Before it returns, the screen menu's unsaved changes are written to a player
/// file that gave its values at the start.
///
/// # Errors
///
/// Returns the [`ScreenFailure`] of a step before the session runs, and of a session that failed.
pub(crate) fn run(manifest: &Path, root: &Path, view: View) -> Result<(), ScreenFailure> {
    let (settings, file) = settings(root);
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
    let mut screen = Screen {
        eyes: None,
        immersive,
        menu: ScreenMenu::new(settings, file, root),
        pointer: Pointer::new(),
        link: Link::new(names),
        session,
        draw,
        device,
        controllers: Controllers::new(),
        first_shown: Once::new(),
    };
    let shown = screen.show_until_end(&mut GameWatch::new(Instant::now()));
    screen.pointer.finish();
    screen.menu.finish();
    shown
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

/// Everything the screen opened. Fields drop in declaration order, so the swapchains and the menu's
/// panels go before the session and the device goes last.
struct Screen {
    eyes: Option<Eyes>,
    /// `None` in the view [`View::Screen`].
    immersive: Option<Immersive>,
    menu: ScreenMenu,
    pointer: Pointer,
    link: Link,
    session: Session,
    draw: RegionDraw,
    device: Device,
    controllers: Controllers,
    first_shown: Once,
}

impl Screen {
    fn show_until_end(&mut self, game: &mut GameWatch) -> Result<(), ScreenFailure> {
        loop {
            if game.ended(Instant::now(), checks::game_running) {
                info!("Steam reports that the game ended, so the screen ends");
                return Ok(());
            }
            let frame = match self.session.poll() {
                Ok(Lifecycle::Ended) => {
                    info!("the runtime ended the session, so the screen ends");
                    return Ok(());
                }
                Ok(Lifecycle::Idle) => {
                    // thread::sleep is banned for game threads. Parking waits the same, and an
                    // early wake only polls again.
                    thread::park_timeout(IDLE_WAIT);
                    Ok(())
                }
                Ok(Lifecycle::Running) => self.frame(),
                Err(error) => Err(FrameError::Xr(error)),
            };
            match frame {
                Ok(()) => {}
                Err(error) if error.is_loss() => {
                    info!(error = %error, "the runtime lost the session, so the screen ends");
                    return Ok(());
                }
                Err(error) => return Err(error.into()),
            }
        }
    }

    /// Waits for the session's next frame, reads the controllers, writes the head pose in the view
    /// [`View::Immersive`], draws the newest published frame into the swapchains when the runtime
    /// shows the frame and chooses its layer, updates the menu, sends the game the pointer's mouse
    /// and turn key input, draws the menu's quads and the laser, and ends the frame with the
    /// frame's layer, then the menu's panels.
    fn frame(&mut self) -> Result<(), FrameError> {
        let timing = self.session.wait_frame()?;
        let controls = self.controllers.read(&self.session, timing)?;
        let now = Instant::now();
        let Self {
            eyes, immersive, menu, pointer, link, session, draw, device, first_shown, ..
        } = self;
        if let Some(immersive) = immersive.as_mut() {
            immersive.write_head(session.locate_head(timing), link)?;
        }
        let drawn =
            if timing.should_render { show(eyes, link, session, draw, device)? } else { None };
        let choice = drawn.as_ref().zip(immersive.as_mut()).map(|(drawn, on)| on.choose(drawn.tag));
        let mut menu_shown = menu.update(&controls, now, layer::pointing(choice.as_ref()));
        let frame_layer = drawn.and_then(|drawn| Layer::of(drawn.frame, choice, menu.screen()));
        let shown = drawn.zip(frame_layer).map(|(drawn, on)| on.shown(drawn.swapchains));
        let panel = if menu_shown.points_at_panel() { PanelAim::Met } else { PanelAim::Missed };
        let screen = frame_layer.and_then(Layer::screen_quad);
        let sight = Sight { menu: menu.hand(), panel, screen };
        if let Some(laser) = pointer.update(&controls, sight, now) {
            menu_shown.laser = Some(laser);
        }
        menu.draw(menu_shown, &Gpu { session, draw, context: device.context() });
        if let Some(Shown::Screen { swapchains, .. } | Shown::Projection { swapchains, .. }) = shown
        {
            let (width, height) = swapchains.size();
            once!(*first_shown, Level::INFO, width, height, "first frame shown in the headset");
        }
        session.end_frame(timing, shown, &menu.panels())?;
        if let Some(immersive) = immersive.as_mut() {
            immersive.end_frame();
        }
        Ok(())
    }
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

/// The screen's reads of Steam's report that the game runs, at most one per
/// [`GAME_CHECK_INTERVAL`].
struct GameWatch {
    next_check: Instant,
    read_failure: Once,
}

impl GameWatch {
    const fn new(now: Instant) -> Self {
        Self { next_check: now, read_failure: Once::new() }
    }

    /// Whether `game_running`, called once the interval passed, reports that the game ended. A
    /// failed read keeps the screen running.
    fn ended(
        &mut self,
        now: Instant,
        game_running: impl FnOnce() -> Result<bool, RegistryError>,
    ) -> bool {
        if now < self.next_check {
            return false;
        }
        self.next_check = now.checked_add(GAME_CHECK_INTERVAL).unwrap_or(now);
        match game_running() {
            Ok(running) => !running,
            Err(error) => {
                once!(
                    self.read_failure,
                    Level::WARN,
                    error = %error,
                    "could not read whether the game runs, so the screen keeps running"
                );
                false
            }
        }
    }
}

#[cfg(test)]
mod tests;
