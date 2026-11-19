//! The screen menu: a panel above one controller whose buttons set the screen's distance and
//! width, and the laser of the other controller that points at them.
//!
//! `model.rs` decides what the menu does in each frame, `rows.rs` holds its rows and their steps,
//! `place.rs` places its two quads and `save.rs` decides when the player file is written. This
//! module paints the panel with the `overlay` crate's painter, draws the panel's pixels and the
//! laser's color into their swapchains, writes the player file and logs what the menu did. A
//! failure of the painter or of a swapchain leaves the screen with no menu for the session.

mod model;
mod place;
mod rows;
mod save;

use std::path::{Path, PathBuf};
use std::time::Instant;

use diag::error_chain;
use gfx::canvas::CanvasError;
use gfx::region::{Placement, RegionDraw, RegionError};
use openxr::{Extent2Df, Posef};
use overlay::layout::{PANEL_HEIGHT, PANEL_WIDTH};
use overlay::menu::TooManyRows;
use overlay::painter::{Change, Painter};
use settings::file::{self, SETTINGS_FILE};
use settings::player::ScreenSettings;
use tracing::{info, warn};
use windows::Win32::Graphics::Direct3D11::{
    ID3D11DeviceContext, ID3D11RenderTargetView, ID3D11ShaderResourceView, ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT;
use xr::error::XrError;
use xr::format::{Sampling, SwapchainFormat};
use xr::panel::Panel;
use xr::session::Session;

use self::model::{Event, Model, Shown};
use self::place::PANEL_SIZE;
pub(crate) use self::save::PlayerFile;
use self::save::{Saver, Saving};
use super::controls::Frame;

/// The side of the laser's swapchain images in pixels, which hold one color.
const LASER_SIDE: u16 = 4;

/// The laser's color, a light blue, as sRGB-encoded red, green, blue and alpha.
const LASER_COLOR: [u8; 4] = [140, 200, 255, 255];

const LASER_PIXELS: [[u8; 4]; 16] = [LASER_COLOR; 16];
const _: () = assert!(LASER_PIXELS.len() == LASER_SIDE as usize * LASER_SIDE as usize);

/// The objects the menu draws with in a frame.
pub(crate) struct Gpu<'a> {
    pub(crate) session: &'a Session,
    pub(crate) draw: &'a RegionDraw,
    pub(crate) context: &'a ID3D11DeviceContext,
}

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
    #[error("the runtime gave a menu quad the swapchain image {index}, which its image list lacks")]
    NoImage { index: u32 },
    #[error("a menu picture of {width} by {height} pixels has no placement")]
    Size { width: u16, height: u16 },
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
    /// `None` once the menu failed or its painter could not be made, so the screen has no menu.
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

    /// Takes the controllers' `frame` at `now` and draws the menu's quads with `gpu`.
    pub(crate) fn update(&mut self, frame: &Frame, now: Instant, gpu: &Gpu<'_>) {
        if self.drawing.is_none() {
            return;
        }
        let drawn = self.take(frame, now).and_then(|shown| self.draw(shown, gpu));
        if let Err(error) = drawn {
            self.fail(&error);
        }
    }

    /// Takes the controllers' `frame` at `now`, logs what the menu did, writes the player file when
    /// the menu closed with changed values, and returns what the menu shows.
    fn take(&mut self, frame: &Frame, now: Instant) -> Result<Shown, MenuError> {
        let mut closes = 0_u32;
        let shown = self.model.update(frame, now, &mut |event| {
            closes = closes.saturating_add(u32::from(matches!(event, Event::Closed(_))));
            log(event);
        });
        if closes > 0 {
            self.save();
        }
        Ok(shown?)
    }

    fn draw(&mut self, shown: Shown, gpu: &Gpu<'_>) -> Result<(), MenuError> {
        let screen = self.model.screen();
        match &mut self.drawing {
            Some(drawing) => drawing.draw(shown, screen, gpu),
            None => Ok(()),
        }
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
    warn!(error = %error, "the screen menu failed, so the screen runs on with no menu");
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

/// A view of each image of `panel` in the format [`target_format`] names.
fn views(panel: &Panel, gpu: &Gpu<'_>) -> Result<Vec<ID3D11RenderTargetView>, RegionError> {
    let format = target_format(gpu.session.format());
    panel.images().iter().map(|image| gpu.draw.target_view(image, format)).collect()
}

/// The view format of a panel image of `format` that stores the values a draw writes, since the
/// pixels of the painter and of the laser are sRGB-encoded.
const fn target_format(format: SwapchainFormat) -> DXGI_FORMAT {
    format.target_view(Sampling::Stored)
}

/// A texture to draw into a panel, as large as the panel's images.
#[derive(Clone, Copy)]
struct Source<'a> {
    texture: &'a ID3D11Texture2D,
    width: u16,
    height: u16,
}

/// A view of `source` to draw, and its placement over an entire target of its size.
fn prepare(
    draw: &RegionDraw,
    source: Source<'_>,
) -> Result<(ID3D11ShaderResourceView, Placement), MenuError> {
    let Source { texture, width, height } = source;
    let placement = Placement::new((0, 0), (u32::from(width), u32::from(height)))
        .ok_or(MenuError::Size { width, height })?;
    Ok((draw.source_view(texture)?, placement))
}

/// Draws `source` into the next image of `panel`, whose views are `targets`, and releases the
/// image.
fn show(
    panel: &mut Panel,
    targets: &[ID3D11RenderTargetView],
    source: Source<'_>,
    gpu: &Gpu<'_>,
) -> Result<(), MenuError> {
    let (view, placement) = prepare(gpu.draw, source)?;
    let index = panel.acquire()?;
    let target = usize::try_from(index).ok().and_then(|slot| targets.get(slot));
    if let Some(target) = target {
        gpu.draw.draw(gpu.context, target, &view, placement);
    }
    panel.release()?;
    match target {
        Some(_) => Ok(()),
        None => Err(MenuError::NoImage { index }),
    }
}

#[cfg(test)]
mod tests;
