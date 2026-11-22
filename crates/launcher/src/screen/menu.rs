//! The screen menu: a panel fixed to one controller whose buttons set the screen's distance and
//! width, and the laser of the other controller that points at them. The caller replaces the laser
//! of [`Shown`] with the laser of the game's pointer while that pointer's ray hits the screen.
//!
//! `model.rs` decides what the menu does in each frame, `rows.rs` holds its rows and their steps,
//! `place.rs` places its two quads and `save.rs` decides when the player file is written. This
//! module paints the panel with the `overlay` crate's painter, draws the panel's pixels and the
//! laser's color into their swapchains, writes the player file and logs what the menu did. A
//! failure of the painter or of a swapchain leaves the screen with no menu and no laser for the
//! session, the game pointer's laser included.

mod model;
mod place;
mod rows;
mod save;

use std::path::{Path, PathBuf};
use std::time::Instant;

use device_profile::profile::Hand;
use diag::error_chain;
use gfx::canvas::CanvasError;
use gfx::region::RegionError;
use openxr::{Extent2Df, Posef};
use overlay::layout::{PANEL_HEIGHT, PANEL_WIDTH};
use overlay::menu::TooManyRows;
use overlay::painter::{Change, Painter};
use settings::file::{self, SETTINGS_FILE};
use settings::player::ScreenSettings;
use tracing::{info, warn};
use windows::Win32::Graphics::Direct3D11::ID3D11RenderTargetView;
use xr::error::XrError;
use xr::panel::Panel;

use self::model::{Event, Model};
pub(crate) use self::model::{Pointing, Shown};
use self::place::PANEL_SIZE;
pub(crate) use self::place::{Laser, laser};
pub(crate) use self::save::PlayerFile;
use self::save::{Saver, Saving};
use super::controls::Frame;
use super::panel_image::{Gpu, PanelImageError, Source, show, views};

/// The side of the laser's swapchain images in pixels, which hold one color.
const LASER_SIDE: u16 = 4;

/// The laser's color, a light blue, as sRGB-encoded red, green, blue and alpha.
const LASER_COLOR: [u8; 4] = [140, 200, 255, 255];

const LASER_PIXELS: [[u8; 4]; 16] = [LASER_COLOR; 16];
const _: () = assert!(LASER_PIXELS.len() == LASER_SIDE as usize * LASER_SIDE as usize);

/// Why the menu stopped.
#[derive(Debug, thiserror::Error)]
enum MenuError {
    #[error("the menu panel could not be painted")]
    Paint(#[from] CanvasError),
    #[error("a menu swapchain failed")]
    Xr(#[from] XrError),
    #[error("a menu picture could not be drawn")]
    Draw(#[from] RegionError),
    #[error("the menu could not be laid out")]
    Rows(#[from] TooManyRows),
    #[error("a menu quad could not be drawn")]
    Image(#[from] PanelImageError),
}

/// Whether a quad is shown in this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Showing {
    Hidden,
    Shown,
}

/// The menu's state, its player file and its drawing.
pub(crate) struct ScreenMenu {
    model: Model,
    saver: Saver,
    root: PathBuf,
    /// `None` once the menu failed or its painter could not be made, so the screen has no menu and
    /// no laser.
    drawing: Option<Drawing>,
}

impl ScreenMenu {
    /// A closed menu over `screen`, the values that the player file in the root folder `root` gave
    /// at the start, or the defaults when `file` says it gave none.
    pub(crate) fn new(screen: ScreenSettings, file: PlayerFile, root: &Path) -> Self {
        let drawing = match Painter::new() {
            Ok(painter) => Some(Drawing::new(painter)),
            Err(error) => {
                warn_failure(&MenuError::Paint(error));
                None
            }
        };
        let saver = Saver::new(screen, file);
        Self { model: Model::new(screen), saver, root: root.into(), drawing }
    }

    /// The screen's values, which the menu's steps change.
    pub(crate) const fn screen(&self) -> ScreenSettings {
        self.model.screen()
    }

    /// The hand the open menu is on, or `None` while the menu is closed or gone.
    pub(crate) const fn hand(&self) -> Option<Hand> {
        self.model.hand()
    }

    /// Takes the controllers' `frame` at `now` while they are `pointing`, and returns what the menu
    /// shows. A menu that is gone shows nothing.
    pub(crate) fn update(&mut self, frame: &Frame, now: Instant, pointing: Pointing) -> Shown {
        if self.drawing.is_none() {
            return Shown::default();
        }
        match self.take(frame, now, pointing) {
            Ok(shown) => shown,
            Err(error) => {
                self.fail(&error);
                Shown::default()
            }
        }
    }

    /// Draws the quads of `shown` with `gpu`.
    pub(crate) fn draw(&mut self, shown: Shown, gpu: &Gpu<'_>) {
        let screen = self.model.screen();
        let Some(drawing) = &mut self.drawing else {
            return;
        };
        if let Err(error) = drawing.draw(shown, screen, gpu) {
            self.fail(&error);
        }
    }

    /// Takes the controllers' `frame` at `now` while they are `pointing`, logs what the menu did,
    /// writes the player file when the menu closed with changed values, and returns what the menu
    /// shows.
    fn take(
        &mut self,
        frame: &Frame,
        now: Instant,
        pointing: Pointing,
    ) -> Result<Shown, MenuError> {
        let mut closes = 0_u32;
        let shown = self.model.update(frame, now, pointing, &mut |event| {
            closes = closes.saturating_add(u32::from(matches!(event, Event::Closed(_))));
            log(event);
        });
        if closes > 0 {
            self.save();
        }
        Ok(shown?)
    }

    /// The panels the frame shows, the menu panel before the laser.
    pub(crate) fn panels(&self) -> Vec<&Panel> {
        let Some(Drawing { panels: Some(panels), menu, laser, .. }) = &self.drawing else {
            return Vec::new();
        };
        [(*menu, &panels.menu), (*laser, &panels.laser)]
            .into_iter()
            .filter_map(|(showing, panel)| (showing == Showing::Shown).then_some(panel))
            .collect()
    }

    /// Writes the player file at the session's end when a change is unsaved, such as the change of
    /// a menu that is still open or a write that failed when the menu closed.
    pub(crate) fn finish(&mut self) {
        self.save();
    }

    /// Writes the changed values to the player file and logs the result. The write runs on the
    /// frame loop's thread, since it runs only when the menu closes or the session ends, and the
    /// runtime reprojects the quads over the frames it takes.
    fn save(&mut self) {
        let root = &self.root;
        match self.saver.save(self.model.screen(), |screen| file::write_screen(root, screen)) {
            Saving::Unchanged => {}
            Saving::Refused => {
                warn!(
                    file = SETTINGS_FILE,
                    "the screen settings are not written, since the settings file gave no values at the start"
                );
            }
            Saving::Written(screen) => {
                let (distance_m, width_m) = (screen.distance_m.meters(), screen.width_m.meters());
                info!(file = SETTINGS_FILE, distance_m, width_m, "screen settings written");
            }
            Saving::Failed(error) => {
                let error = error_chain::chain(&error);
                warn!(error = %error, "the screen settings could not be written");
            }
        }
    }

    fn fail(&mut self, error: &MenuError) {
        warn_failure(error);
        self.drawing = None;
        if let Some(hand) = self.model.close() {
            log(Event::Closed(hand));
            self.save();
        }
    }
}

fn warn_failure(error: &MenuError) {
    let error = error_chain::chain(error);
    warn!(error = %error, "the screen menu failed, so the screen runs on with no menu and no laser");
}

/// Logs `event` as one line. A plain `info!` fits the frame loop here, since an event comes from a
/// press or its repeat and never from each frame.
fn log(event: Event) {
    match event {
        Event::Opened(hand) => info!(hand = hand.as_str(), "screen menu opened"),
        Event::Moved(hand) => info!(hand = hand.as_str(), "screen menu moved"),
        Event::Closed(hand) => info!(hand = hand.as_str(), "screen menu closed"),
        Event::Stepped(screen) => {
            let (distance_m, width_m) = (screen.distance_m.meters(), screen.width_m.meters());
            info!(distance_m, width_m, "screen menu step");
        }
    }
}

/// The painter, the swapchains once the menu first showed a quad, and the quads of this frame.
struct Drawing {
    painter: Painter,
    panels: Option<Panels>,
    menu: Showing,
    laser: Showing,
}

impl Drawing {
    const fn new(painter: Painter) -> Self {
        Self { painter, panels: None, menu: Showing::Hidden, laser: Showing::Hidden }
    }

    /// Draws the quads of `shown`, a frame of the menu over the screen values `screen`, with `gpu`.
    /// The menu panel is drawn again only when the painter drew new pixels.
    fn draw(
        &mut self,
        shown: Shown,
        screen: ScreenSettings,
        gpu: &Gpu<'_>,
    ) -> Result<(), MenuError> {
        (self.menu, self.laser) = (Showing::Hidden, Showing::Hidden);
        let Shown { panel, laser } = shown;
        if panel.is_none() && laser.is_none() {
            return Ok(());
        }
        let panels = match &mut self.panels {
            Some(panels) => panels,
            None => self.panels.insert(Panels::create(gpu)?),
        };
        if let Some(panel) = panel {
            let painted = self.painter.paint(&rows::menu(screen)?, panel.point, panel.press)?;
            if painted.change == Change::Redrawn {
                let (pixels, _) = painted.pixels.as_chunks::<4>();
                let (width, height) = (PANEL_WIDTH, PANEL_HEIGHT);
                let texture = gpu.draw.image(width, height, pixels)?;
                show(
                    &mut panels.menu,
                    &panels.targets,
                    Source { texture: &texture, width, height },
                    gpu,
                )?;
            }
            panels.menu.pose = panel.pose;
            self.menu = Showing::Shown;
        }
        if let Some(laser) = laser {
            (panels.laser.pose, panels.laser.size) = (laser.pose, laser.size);
            self.laser = Showing::Shown;
        }
        Ok(())
    }
}

/// The menu panel with a view of each of its images, and the laser. Fields drop in declaration
/// order, so the views go before the panel.
struct Panels {
    targets: Vec<ID3D11RenderTargetView>,
    menu: Panel,
    laser: Panel,
}

impl Panels {
    /// Creates the menu panel and the laser, and draws the laser's color into the laser.
    fn create(gpu: &Gpu<'_>) -> Result<Self, MenuError> {
        let menu_pixels = (u32::from(PANEL_WIDTH), u32::from(PANEL_HEIGHT));
        let menu = gpu.session.create_panel(menu_pixels, Posef::IDENTITY, PANEL_SIZE)?;
        let targets = views(&menu, gpu)?;
        let laser_pixels = (u32::from(LASER_SIDE), u32::from(LASER_SIDE));
        let laser_size = Extent2Df { width: 1.0, height: 1.0 };
        let mut laser = gpu.session.create_panel(laser_pixels, Posef::IDENTITY, laser_size)?;
        let (width, height) = (LASER_SIDE, LASER_SIDE);
        let texture = gpu.draw.image(width, height, &LASER_PIXELS)?;
        let laser_targets = views(&laser, gpu)?;
        show(&mut laser, &laser_targets, Source { texture: &texture, width, height }, gpu)?;
        Ok(Self { targets, menu, laser })
    }
}

#[cfg(test)]
mod tests;
