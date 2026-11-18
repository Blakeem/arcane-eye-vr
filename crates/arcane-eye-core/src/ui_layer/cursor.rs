//! The drawn cursor: the game's mouse cursor drawn into each half of the back buffer after the UI,
//! with the Windows cursor hidden over the game window's client area in each frame that draws the
//! cursor and whose step hides it.
//!
//! ```text
//! UI LAYER FRAME, render thread, while the feature runs
//!   [owner]     the thread of the swapchain's output window, against the calling thread
//!   [aim]       the pointer in the window's client area and the calling thread's cursor
//!   [image]     the cursor's bitmaps as pixels and a texture, made again only when its handle changes
//!   [draw]      the texture with its hotspot at the pointer's place in each half, clipped to the half
//!   [counter]   one ShowCursor(FALSE) when a frame draws the cursor and hides the Windows cursor.
//!               One ShowCursor(TRUE) when a frame shows it again, such as a frame with no drawn
//!               cursor or a frame the frame handler skips, or after the feature stops
//! ```
//!
//! The game shows its pointer through the Windows cursor, and the UI texture holds no cursor (both
//! measured in the game). A fault of a cursor's read, conversion or placed draw skips that cursor
//! until the handle changes and never stops the UI layer.

pub(crate) mod counter;
pub(crate) mod image;
pub(crate) mod place;

use diag::rate::{Once, RateLimit};
use diag::{once, warn_every};
use gfx::layer::{LayerDraw, LayerError, Rect};
use tracing::Level;
use windows::Win32::Graphics::Direct3D11::{ID3D11DeviceContext, ID3D11Texture2D};

use self::counter::{Counter, Marked, Owner, WindowsCursor, owner, wanted};
use self::image::{ConvertError, Image};
use self::place::{Spot, Sprite, place, spot};
use super::LOG_PERIOD;
use crate::raw::cursor::{
    self as raw_cursor, CursorBitmaps, CursorHandle, ReadFault, ReleaseLogs, Window,
};

/// The cursor of one frame and the pointer's spot in the client area.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Aim {
    pub(crate) handle: CursorHandle,
    pub(crate) spot: Spot,
}

/// The frame the cursor is drawn into: the back buffer, its device's immediate context and the
/// back buffer's halves.
#[derive(Clone, Copy)]
pub(crate) struct Canvas<'frame> {
    pub(crate) context: &'frame ID3D11DeviceContext,
    pub(crate) back_buffer: &'frame ID3D11Texture2D,
    pub(crate) halves: &'frame [Rect; 2],
}

/// Why a cursor is skipped until its handle changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub(crate) enum CursorFault {
    #[error(transparent)]
    Read(#[from] ReadFault),
    #[error(transparent)]
    Convert(#[from] ConvertError),
    #[error(transparent)]
    Layer(#[from] LayerError),
}

const _: () = assert!(size_of::<CursorFault>() <= 16, "a per-frame error holds at most 16 bytes");

/// The texture of a cursor image and its size and hotspot.
struct Drawn {
    texture: ID3D11Texture2D,
    sprite: Sprite,
}

/// The last cursor handle and its texture, or `None` for a cursor that is skipped.
struct Held {
    handle: CursorHandle,
    drawn: Option<Drawn>,
}

/// The drawn cursor's render thread state, held in the UI layer.
#[derive(Default)]
pub(crate) struct Cursor {
    counter: Counter,
    held: Option<Held>,
    other_thread_log: Once,
    gone_log: Once,
    pointer_log: RateLimit,
    skip_log: RateLimit,
    release_logs: ReleaseLogs,
}

impl Cursor {
    /// The cursor to draw into this frame of the game window `window`, or `None` when no cursor is
    /// drawn, such as for a pointer outside the window's client area.
    pub(crate) fn aim(&mut self, window: Window) -> Option<Aim> {
        if self.owner(window) != Owner::Caller {
            return None;
        }
        let pointer = match raw_cursor::pointer(window) {
            Ok(pointer) => pointer?,
            Err(fault) => {
                warn_every!(self.pointer_log, LOG_PERIOD, fault = %fault, "cursor pointer read failed");
                return None;
            }
        };
        let spot = spot(pointer)?;
        let handle = raw_cursor::current_cursor()?;
        Some(Aim { handle, spot })
    }

    /// Draws the cursor of `aim` into each half of `canvas` with `layer`. A cursor that cannot be
    /// converted or drawn is logged and skipped until its handle changes.
    pub(crate) fn draw(&mut self, layer: &LayerDraw, canvas: &Canvas<'_>, aim: Aim) -> Marked {
        let Some(drawn) = self.drawn(layer, aim.handle) else { return Marked::Nothing };
        let placements = place(aim.spot, drawn.sprite, canvas.halves);
        let Canvas { context, back_buffer, .. } = *canvas;
        if let Err(error) = layer.draw_placed(context, back_buffer, &drawn.texture, &placements) {
            self.skip(aim.handle, error.into());
            self.held = Some(Held { handle: aim.handle, drawn: None });
            return Marked::Nothing;
        }
        Marked::Drawn
    }

    /// Hides the Windows cursor on the game window's thread, or shows it again, after a frame whose
    /// step wants `want` and that drew `marked`.
    pub(crate) fn mark(&mut self, want: WindowsCursor, marked: Marked) {
        // Only a drawn cursor hides the Windows cursor, and `aim` aims only on the window's thread,
        // so the counter of another thread is never lowered.
        self.count(wanted(want, marked));
    }

    /// Gives the cursor display counter the game's value back, so the Windows cursor shows in a
    /// frame that draws no cursor. Keeps the cursor's texture for the next frame.
    pub(crate) fn show(&mut self) {
        self.count(WindowsCursor::Shown);
    }

    /// Drops the cursor's texture, which belongs to the device of the layer draw that made it.
    pub(crate) fn forget_texture(&mut self) {
        self.held = None;
    }

    /// Drops the cursor's texture and gives the cursor display counter the game's value back. Runs
    /// every frame after the feature stops.
    pub(crate) fn release(&mut self) {
        self.held = None;
        self.show();
    }

    /// The texture of the cursor `handle`, made from its bitmaps when the handle differs from the
    /// last one.
    fn drawn(&mut self, layer: &LayerDraw, handle: CursorHandle) -> Option<&Drawn> {
        if self.held.as_ref().is_none_or(|held| held.handle != handle) {
            let drawn = match texture(layer, handle, &mut self.release_logs) {
                Ok(drawn) => Some(drawn),
                Err(fault) => {
                    self.skip(handle, fault);
                    None
                }
            };
            self.held = Some(Held { handle, drawn });
        }
        self.held.as_ref()?.drawn.as_ref()
    }

    /// The thread that owns `window`. Logs once when the calling thread does not own it.
    fn owner(&mut self, window: Window) -> Owner {
        let caller = raw_cursor::current_thread();
        let owner = owner(caller, raw_cursor::window_thread(window));
        match owner {
            Owner::Caller => {}
            Owner::Other(window_thread) => {
                once!(
                    self.other_thread_log,
                    Level::WARN,
                    caller,
                    window_thread,
                    "the game window belongs to another thread, so the core draws no cursor and \
                     does not hide the Windows cursor"
                );
            }
            Owner::Gone => {
                once!(
                    self.gone_log,
                    Level::WARN,
                    caller,
                    "the swapchain's output window handle names no window, so the core draws no \
                     cursor and does not hide the Windows cursor"
                );
            }
        }
        owner
    }

    /// Makes the counter call that a frame wanting `windows_cursor` needs.
    fn count(&mut self, windows_cursor: WindowsCursor) {
        if let Some(call) = counter::call(self.counter, windows_cursor) {
            raw_cursor::show(call);
            self.counter = call.counter();
        }
    }

    /// Logs the cursor `handle` that `fault` skips. The caller logs once per handle change, and the
    /// rate limit bounds a pointer that moves between failing cursors.
    fn skip(&mut self, handle: CursorHandle, fault: CursorFault) {
        warn_every!(
            self.skip_log,
            LOG_PERIOD,
            handle = format_args!("{:#x}", handle.get()),
            fault = %fault,
            "cursor skipped until it changes"
        );
    }
}

/// The texture of the cursor `handle`. A failed release of a GDI object logs through `logs`.
fn texture(
    layer: &LayerDraw,
    handle: CursorHandle,
    logs: &mut ReleaseLogs,
) -> Result<Drawn, CursorFault> {
    let image = read(handle, logs)?;
    let texture = layer.image(image.width, image.height, &image.pixels)?;
    let sprite = Sprite { width: image.width, height: image.height, hotspot: image.hotspot };
    Ok(Drawn { texture, sprite })
}

/// The image of the cursor `handle`, from its bitmaps. A failed release of a GDI object logs
/// through `logs`.
pub(crate) fn read(handle: CursorHandle, logs: &mut ReleaseLogs) -> Result<Image, CursorFault> {
    let mut bitmaps = CursorBitmaps::of(handle, logs)?;
    let shape = image::shape(bitmaps.sizes()?)?;
    let mask = bitmaps.mask_rows(shape)?;
    let color = bitmaps.color_rows(shape)?;
    Ok(image::convert(shape, bitmaps.hotspot(), &mask, color.as_deref())?)
}
