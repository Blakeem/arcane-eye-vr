//! The frame the UI layer draws into at the Present call site, and why a draw into it failed.

use gfx::layer::LayerError;
use windows::Win32::Graphics::Dxgi::IDXGISwapChain;
use windows::core::HRESULT;

use crate::game_source::SourceFault;
use crate::raw::cursor::Window;
use crate::raw::{BackBuffer, Devices};

/// Why a frame's pair, move, UI texture or cursor was not drawn: a fault of the back buffer, of the
/// UI texture's read or of the layer draw. It stops the UI layer only in a frame that draws the
/// pair, the move or the UI texture. A fault of the cursor's own read, conversion or placed draw
/// skips only the cursor and is not one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub(crate) enum DrawFault {
    #[error("the back buffer's size {width} by {height} holds no rectangle")]
    Size { width: u32, height: u32 },
    #[error(transparent)]
    Source(#[from] SourceFault),
    #[error("the back buffer could not be taken: {code}")]
    BackBuffer { code: HRESULT },
    #[error(transparent)]
    Layer(#[from] LayerError),
}

/// The back buffer of the frame at the Present call site, and the swapchain's output window.
pub(crate) struct Target<'frame> {
    pub(crate) swapchain: &'frame IDXGISwapChain,
    pub(crate) devices: &'frame Devices,
    pub(crate) size: BackBuffer,
    pub(crate) window: Window,
}
