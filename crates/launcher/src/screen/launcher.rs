//! The Larian launcher's window in the headset during the start wait: the newest copy of its
//! client area on a panel at the screen quad's place, which both eyes see in place of the status
//! panel, and the window the pointer's input goes to while the copy shows.
//!
//! `watch.rs` decides when the launch app looks for the window and asks for its copy, and what the
//! pointer acts on. `copier.rs` copies the window on a thread of its own, since a copy from the
//! screen takes longer than one frame. This module searches for the window, draws each finished
//! copy into the panel and logs what the watch found. A failed copy logs a rate-limited warning,
//! hides the picture and leaves the start running. The copy of a window that went hidden or
//! minimized hides the picture with no warning. At the goal status the panel goes. A window the
//! watch lets go, at the goal status, at the session's end, when a search lost it or found another
//! window in its place, loses the topmost order its copies gave it and goes below every other
//! window, so the game window lies above it. The copy thread lives until the session's end.

mod copier;
mod watch;

use std::io;
use std::time::{Duration, Instant};

use diag::error_chain;
use diag::rate::RateLimit;
use diag::warn_every;
use gfx::region::RegionError;
use launch::launcher_window::{self, LauncherWindow};
use launch::window_copy::{WindowCopyError, WindowPicture};
use settings::player::ScreenSettings;
use tracing::info;
use windows::Win32::Graphics::Direct3D11::ID3D11RenderTargetView;
use xr::error::XrError;
use xr::panel::Panel;
use xr::quad::ScreenQuad;

use self::copier::{Copier, Stopped};
use self::watch::{Change, Watch};
pub(crate) use self::watch::{Target, Wait, target};
use super::layer;
use super::panel_image::{self, Gpu, PanelImageError, Source};

/// The shortest time between two warnings of a failed copy of the launcher window.
const FAILURE_PERIOD: Duration = Duration::from_secs(10);

/// Why the launcher window's copy did not show.
#[derive(Debug, thiserror::Error)]
pub(crate) enum LauncherError {
    #[error("the launcher window could not be copied")]
    Copy(#[from] WindowCopyError),
    #[error("the launcher window's copy thread could not start")]
    Thread(#[from] io::Error),
    #[error("the launcher window's copy thread ended, so no copy was asked")]
    Stopped(#[from] Stopped),
    #[error("the launcher window's copy of {width} by {height} pixels fits no screen quad")]
    Size { width: u32, height: u32 },
    #[error("the launcher's panel swapchain failed")]
    Xr(#[from] XrError),
    #[error("the launcher's picture could not be made")]
    Draw(#[from] RegionError),
    #[error("the launcher's picture could not be drawn")]
    Image(#[from] PanelImageError),
}

/// A copy of the launcher window's client area as red, green, blue and an opaque alpha, made on
/// the copy thread.
struct Pixels {
    width: u32,
    height: u32,
    rgba: Vec<[u8; 4]>,
}

/// The copy thread's result for one window.
type Copied = Result<Pixels, WindowCopyError>;

/// The panel that shows the launcher's picture, a view of each of its images and the picture's
/// size. Fields drop in declaration order, so the views go before the panel.
struct Picture {
    targets: Vec<ID3D11RenderTargetView>,
    panel: Panel,
    size: (u32, u32),
}

/// The launcher's window in the headset across the frames of a session.
pub(crate) struct LauncherScreen {
    watch: Watch<LauncherWindow>,
    /// `None` until the first copy is asked for.
    copier: Option<Copier<LauncherWindow, Copied>>,
    picture: Option<Picture>,
    failure: RateLimit,
}

impl LauncherScreen {
    pub(crate) fn new() -> Self {
        Self { watch: Watch::new(), copier: None, picture: None, failure: RateLimit::new() }
    }

    /// Takes the frame at `now` of the start at `wait`: draws the copy the copy thread finished
    /// into its panel with `gpu`, searches for the launcher window and asks for its copy as the
    /// watch decides, and places the panel at the screen quad of `settings` for the picture's size.
    pub(crate) fn update(
        &mut self,
        now: Instant,
        wait: Wait,
        settings: ScreenSettings,
        gpu: &Gpu<'_>,
    ) {
        let Self { watch, copier, picture, failure } = self;
        if let Some((window, copied)) = copier.as_mut().and_then(Copier::take) {
            let draw = || show(copied, picture, settings, gpu);
            warn_failed(failure, watch.copied(&window, draw));
        }
        let ask = |window: &LauncherWindow| request(copier, window);
        let step = watch.update(now, wait, launcher_window::find, ask);
        // A plain `info!` fits the frame loop here, since the window changes at most once per
        // search and never from each frame.
        match (step.change, watch.window()) {
            (Some(Change::Found), Some(window)) => {
                let (width, height) = window.size();
                info!(width, height, "the launcher window was found");
            }
            (Some(Change::Lost), _) => info!("the launcher window was lost"),
            (Some(Change::Found), None) | (None, _) => {}
        }
        warn_failed(failure, step.failure);
        if let Some(window) = step.released {
            release(copier.as_ref(), window);
        }
        match (wait, picture.as_mut()) {
            (Wait::Done, _) => *picture = None,
            (Wait::Running, Some(shown)) => place(shown, settings),
            (Wait::Running, None) => {}
        }
    }

    /// The screen quad the launcher's picture shows on and the window it shows, while it shows.
    pub(crate) fn shown(&self) -> Option<(ScreenQuad, &LauncherWindow)> {
        let window = self.watch.shown()?;
        let picture = self.picture.as_ref()?;
        let quad = ScreenQuad { pose: picture.panel.pose, size: picture.panel.size };
        Some((quad, window))
    }

    /// The panel of the launcher's picture, while it shows.
    pub(crate) fn panel(&self) -> Option<&Panel> {
        self.watch.shown()?;
        self.picture.as_ref().map(|picture| &picture.panel)
    }

    /// Lets the launcher window go at the session's end, so it loses the topmost order, and ends
    /// the copy thread once it ran every request.
    pub(crate) fn finish(&mut self) {
        if let Some(window) = self.watch.end() {
            release(self.copier.as_ref(), window);
        }
        drop(self.copier.take());
    }
}

/// Asks the copy thread in `slot` for a copy of `window`, after it starts the thread when none
/// runs.
fn request(
    slot: &mut Option<Copier<LauncherWindow, Copied>>,
    window: &LauncherWindow,
) -> Result<(), LauncherError> {
    let copier = match slot {
        Some(copier) => copier,
        None => slot.insert(Copier::start(copy, end_topmost)?),
    };
    copier.copy(window)?;
    Ok(())
}

/// Asks the copy thread `copier` to end the topmost order of `window` after its copies, or ends
/// it here when no copy thread runs.
fn release(copier: Option<&Copier<LauncherWindow, Copied>>, window: LauncherWindow) {
    let sent = match copier {
        Some(copier) => copier.release(window),
        None => Err(window),
    };
    if let Err(unsent) = sent {
        end_topmost(&unsent);
    }
}

/// Warns of `failure` at most once per [`FAILURE_PERIOD`], apart from the copy of a hidden or
/// minimized window, which the next search loses with no warning, since it is no launcher window.
fn warn_failed(period: &mut RateLimit, failure: Option<LauncherError>) {
    if let Some(error) = failure.filter(|error| !is_hidden(error)) {
        warn_every!(
            *period,
            FAILURE_PERIOD,
            error = %error_chain::chain(&error),
            "the launcher window's copy failed, so the headset shows the status panel"
        );
    }
}

/// Copies `window` on the copy thread.
fn copy(window: &LauncherWindow) -> Copied {
    let copy = window.copy()?;
    Ok(Pixels { width: copy.width, height: copy.height, rgba: rgba(&copy) })
}

/// Draws the finished copy `copied` into the panel in `slot`, after it replaces the panel when
/// its size no longer fits the copy. A new panel sits at the screen quad of `settings`.
fn show(
    copied: Copied,
    slot: &mut Option<Picture>,
    settings: ScreenSettings,
    gpu: &Gpu<'_>,
) -> Result<(), LauncherError> {
    let pixels = copied?;
    let size = (pixels.width, pixels.height);
    let too_large = || LauncherError::Size { width: pixels.width, height: pixels.height };
    let (Ok(width), Ok(height)) = (u16::try_from(pixels.width), u16::try_from(pixels.height))
    else {
        return Err(too_large());
    };
    let texture = gpu.draw.image(width, height, &pixels.rgba)?;
    let picture = match slot.take() {
        Some(picture) if picture.size == size => picture,
        stale => {
            // The old swapchain is destroyed before the runtime creates its replacement.
            drop(stale);
            let quad = layer::quad_of(size, settings).ok_or_else(too_large)?;
            let panel = gpu.session.create_panel(size, quad.pose, quad.size)?;
            Picture { targets: panel_image::views(&panel, gpu)?, panel, size }
        }
    };
    let picture = slot.insert(picture);
    let source = Source { texture: &texture, width, height };
    panel_image::show(&mut picture.panel, &picture.targets, source, gpu)?;
    Ok(())
}

/// The pixels of `copy` as red, green, blue and an opaque alpha.
fn rgba(copy: &WindowPicture) -> Vec<[u8; 4]> {
    let (pixels, _no_remainder) = copy.rgb.as_chunks::<3>();
    pixels.iter().map(|&[red, green, blue]| [red, green, blue, u8::MAX]).collect()
}

/// Places the panel of `picture` at the screen quad of `settings`, which the screen menu changes.
fn place(picture: &mut Picture, settings: ScreenSettings) {
    if let Some(quad) = layer::quad_of(picture.size, settings) {
        picture.panel.pose = quad.pose;
        picture.panel.size = quad.size;
    }
}

/// Whether `error` is the copy of a window that went hidden or minimized since the search.
const fn is_hidden(error: &LauncherError) -> bool {
    matches!(error, LauncherError::Copy(WindowCopyError::Hidden { .. }))
}

/// Ends the topmost order of `window` and places it below every other window. A window that closed
/// since the last search fails the call, so the failure is logged as information.
fn end_topmost(window: &LauncherWindow) {
    if let Err(error) = window.end_topmost() {
        let error = error_chain::chain(&error);
        info!(error = %error, "the launcher window's topmost order did not end");
    }
}

#[cfg(test)]
mod tests {
    use launch::window_copy::{WindowCopyError, WindowPicture};

    use super::{LauncherError, is_hidden, rgba};

    #[test]
    fn each_copied_pixel_keeps_its_red_green_and_blue_and_becomes_opaque() {
        let copy = WindowPicture { width: 2, height: 1, rgb: vec![10, 20, 30, 40, 50, 60] };
        assert_eq!(rgba(&copy), [[10, 20, 30, 255], [40, 50, 60, 255]]);
    }

    #[test]
    fn only_the_copy_of_a_hidden_window_counts_as_hidden() {
        let hidden = LauncherError::Copy(WindowCopyError::Hidden { title: "Launcher".to_owned() });
        let failed = LauncherError::Copy(WindowCopyError::Awareness);
        let too_large = LauncherError::Size { width: 70_000, height: 600 };
        assert_eq!(
            (is_hidden(&hidden), is_hidden(&failed), is_hidden(&too_large)),
            (true, false, false)
        );
    }
}
