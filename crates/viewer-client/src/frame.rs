//! The frame read: the published frame, or one region of it, copied back to memory, for a caller
//! that saves it.
//!
//! The core publishes only while a heartbeat advances, so a caller starts [`crate::heartbeat`] before
//! it reads. The reader keeps its device and the opened texture between reads.

use std::num::NonZeroU32;
use std::thread;
use std::time::{Duration, Instant};

use ipc_layout::viewer_header::{AdapterLuid, ViewerHeader};
use tracing::warn;
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_B8G8R8A8_UNORM_SRGB,
    DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_FORMAT_R8G8B8A8_UNORM_SRGB, DXGI_FORMAT_R10G10B10A2_UNORM,
};
use windows::core::HSTRING;

use crate::header::{HeaderReadError, Names, read_header};
use crate::raw::Staging;
use crate::region::Region;
use crate::{Copied, Device, GpuError, HeaderView, SharedTexture};

/// The wait between two looks at the header. It is shorter than one frame at 60 frames per second.
const POLL: Duration = Duration::from_millis(5);

/// The wait after a region read's look that finds no new frame. The read looks again at once, since
/// a wait can last a 15.6 ms timer tick and the game can present a frame every 8.9 ms.
const REGION_POLL: Duration = Duration::ZERO;

/// How to read the 4 bytes of one pixel. The read copies the texture's bytes unchanged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PixelFormat {
    /// `DXGI_FORMAT_R8G8B8A8_UNORM`: the bytes red, green, blue, alpha.
    Rgba8,
    /// `DXGI_FORMAT_R8G8B8A8_UNORM_SRGB`: the bytes red, green, blue, alpha, with sRGB-encoded color.
    Rgba8Srgb,
    /// `DXGI_FORMAT_B8G8R8A8_UNORM`: the bytes blue, green, red, alpha.
    Bgra8,
    /// `DXGI_FORMAT_B8G8R8A8_UNORM_SRGB`: the bytes blue, green, red, alpha, with sRGB-encoded color.
    Bgra8Srgb,
    /// `DXGI_FORMAT_R10G10B10A2_UNORM`: one little-endian 32-bit word, with red in bits 0 to 9, green
    /// in bits 10 to 19, blue in bits 20 to 29 and alpha in bits 30 and 31.
    Rgb10A2,
}

impl PixelFormat {
    /// The pixel format of a texture of `format`, or `None` for a format the frame read does not
    /// support.
    pub const fn from_dxgi(format: DXGI_FORMAT) -> Option<Self> {
        match format {
            DXGI_FORMAT_R8G8B8A8_UNORM => Some(Self::Rgba8),
            DXGI_FORMAT_R8G8B8A8_UNORM_SRGB => Some(Self::Rgba8Srgb),
            DXGI_FORMAT_B8G8R8A8_UNORM => Some(Self::Bgra8),
            DXGI_FORMAT_B8G8R8A8_UNORM_SRGB => Some(Self::Bgra8Srgb),
            DXGI_FORMAT_R10G10B10A2_UNORM => Some(Self::Rgb10A2),
            _ => None,
        }
    }
}

/// One published frame in memory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub format: PixelFormat,
    /// `height` rows of `width` pixels of 4 bytes each, top row first, with no padding between rows.
    pub bytes: Vec<u8>,
}

/// The kept pixels of one region of a published frame, and the frame's present number.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegionFrame {
    /// The count of frames the core's publisher handled up to this frame, published or not, continued
    /// from the header an earlier core left.
    pub present: u32,
    pub pixels: Frame,
}

/// Why a frame read failed.
#[derive(Debug, thiserror::Error)]
pub enum ReadError {
    #[error(transparent)]
    Header(#[from] HeaderReadError),
    #[error(transparent)]
    Gpu(#[from] GpuError),
    /// The shared texture has this raw `DXGI_FORMAT` value, which [`PixelFormat`] does not cover.
    #[error("the shared texture's DXGI format {0} is not RGBA8, BGRA8 or R10G10B10A2")]
    UnsupportedFormat(u32),
    /// No new frame arrived within the caller's timeout.
    #[error("no new frame arrived within {0:?}")]
    Timeout(Duration),
    /// The region holds no pixel or does not lie inside the shared texture of this size.
    #[error("the region {region} does not lie inside the {width}x{height} frame")]
    RegionOutside { region: Region, width: u32, height: u32 },
}

/// Reads published frames from the viewer link.
pub struct FrameReader {
    names: Names,
    header: Option<HeaderView>,
    /// Drops before the device it was opened on.
    opened: Option<Opened>,
    device: Option<Device>,
}

impl FrameReader {
    pub const fn new(names: Names) -> Self {
        Self { names, header: None, opened: None, device: None }
    }

    /// Waits up to `timeout` for a new frame and returns it. A new frame has a header frame counter
    /// that differs from the counter of the frame this reader returned before. The counter the
    /// header held when the reader opened the shared texture also counts as returned, since the core
    /// may not have copied that frame into a texture it just created.
    ///
    /// The bytes are the frame the counter numbers or a later one, since the core may publish again
    /// between the header read and the copy. A keyed mutex the core abandoned reopens the texture.
    /// A shared texture that fails to open is opened again at the next look, since the core drops
    /// the old texture before it shares the new one.
    ///
    /// # Errors
    ///
    /// Returns [`ReadError::Timeout`] when no new frame arrived in time, and [`ReadError::Gpu`] with
    /// the failed open when the last look before the timeout could not open the shared texture.
    /// Returns every other [`ReadError`] at once.
    pub fn read(&mut self, timeout: Duration) -> Result<Frame, ReadError> {
        self.wait(timeout, POLL, |reader| {
            reader.look(|device, opened, _view| entire_frame(device, opened))
        })
    }

    /// Waits up to `timeout` for a new frame, as [`Self::read`] does, and returns its present
    /// number and every `step`-th pixel of every `step`-th row of `region`, starting at the region's
    /// top left pixel. Only the region is copied out of the shared texture. The present number is
    /// read while this process holds the keyed mutex, so it numbers the copied pixels.
    ///
    /// # Errors
    ///
    /// Returns [`ReadError::RegionOutside`] when `region` does not lie inside the shared texture,
    /// and every other error as [`Self::read`] does.
    pub fn read_region(
        &mut self,
        timeout: Duration,
        region: Region,
        step: NonZeroU32,
    ) -> Result<RegionFrame, ReadError> {
        self.wait(timeout, REGION_POLL, |reader| {
            reader.look(|device, opened, view| region_frame(device, opened, view, region, step))
        })
    }

    /// Waits up to `timeout` for the header to describe a shared texture and returns the texture's
    /// width and height.
    ///
    /// # Errors
    ///
    /// Returns [`ReadError::Timeout`] when the header described no texture in time, and a
    /// [`ReadError::Header`] at once.
    pub fn frame_size(&mut self, timeout: Duration) -> Result<(u32, u32), ReadError> {
        self.wait(timeout, POLL, |reader| {
            let (_view, header) = read_header(&mut reader.header, &reader.names.mapping)?;
            let texture = header.texture();
            let size = (texture.width, texture.height);
            Ok(if texture.width == 0 { Look::NoFrame } else { Look::Frame(size) })
        })
    }

    /// Looks with `look` until it finds what it looks for or `timeout` passed. The wait between two
    /// looks is [`retry_wait`].
    fn wait<T>(
        &mut self,
        timeout: Duration,
        frame_poll: Duration,
        mut look: impl FnMut(&mut Self) -> Result<Look<T>, ReadError>,
    ) -> Result<T, ReadError> {
        let start = Instant::now();
        loop {
            let open_failure = match look(self)? {
                Look::Frame(found) => return Ok(found),
                Look::NoFrame => None,
                Look::OpenFailed(error) => Some(error),
            };
            let remaining = timeout.saturating_sub(start.elapsed());
            if remaining.is_zero() {
                return Err(open_failure.map_or(ReadError::Timeout(timeout), ReadError::Gpu));
            }
            let poll = retry_wait(open_failure.as_ref(), frame_poll);
            // thread::sleep is banned for game threads. Parking waits the same, and an early wake
            // only looks early.
            thread::park_timeout(remaining.min(poll));
        }
    }

    /// One look at the header, and the `copy` of a new frame when the header numbers one. `copy`
    /// returns `None` when the core held the shared texture past the wait.
    fn look<T>(
        &mut self,
        copy: impl FnOnce(&Device, &mut Opened, &HeaderView) -> Result<Option<T>, ReadError>,
    ) -> Result<Look<T>, ReadError> {
        let Self { names, header: header_slot, opened: opened_slot, device: device_slot } = self;
        let (view, header) = read_header(header_slot, &names.mapping)?;
        let texture = header.texture();
        if texture.width == 0 {
            return Ok(Look::NoFrame);
        }
        let device = device_on(device_slot, opened_slot, texture.adapter_luid)?;
        let opened = match Opened::open(opened_slot, device, &names.texture, &header)? {
            Ok(opened) => opened,
            Err(error) => return Ok(Look::OpenFailed(error)),
        };
        let frame = header.frame();
        if opened.consumed == frame {
            return Ok(Look::NoFrame);
        }
        match copy(device, opened, view) {
            Ok(Some(read)) => {
                opened.consumed = frame;
                Ok(Look::Frame(read))
            }
            Ok(None) => Ok(Look::NoFrame),
            Err(ReadError::Gpu(GpuError::Abandoned)) => {
                warn!(
                    generation = header.generation(),
                    "the core abandoned the shared texture's keyed mutex, reopening the texture"
                );
                *opened_slot = None;
                Ok(Look::NoFrame)
            }
            // A failed copy, release or read-back may come from a removed device, which the next
            // look replaces.
            Err(ReadError::Gpu(error)) => {
                *opened_slot = None;
                *device_slot = None;
                Err(error.into())
            }
            Err(error) => Err(error),
        }
    }
}

/// The entire shared texture of `opened` copied and read back, or `None` when the core held it
/// past the wait.
fn entire_frame(device: &Device, opened: &Opened) -> Result<Option<Frame>, ReadError> {
    if device.copy(&opened.shared, opened.staging.texture())? == Copied::CoreBusy {
        return Ok(None);
    }
    let bytes = device.read_back(&opened.staging, NonZeroU32::MIN)?;
    let (width, height) = opened.shared.size();
    Ok(Some(Frame { width, height, format: opened.format, bytes }))
}

/// The kept pixels of `region` of the shared texture of `opened` and the present number `view`
/// holds while the copy holds the keyed mutex, or `None` when the core held it past the wait.
fn region_frame(
    device: &Device,
    opened: &mut Opened,
    view: &HeaderView,
    region: Region,
    step: NonZeroU32,
) -> Result<Option<RegionFrame>, ReadError> {
    let (width, height) = opened.shared.size();
    if !region.lies_inside(width, height) {
        return Err(ReadError::RegionOutside { region, width, height });
    }
    let size = (region.width, region.height);
    let staging = match opened.region.take() {
        Some(staging) if staging.size() == size => staging,
        stale => {
            drop(stale);
            device.create_staging(&opened.shared, size)?
        }
    };
    let staging = opened.region.insert(staging);
    let copied =
        device.copy_region(&opened.shared, staging.texture(), region, || view.read_present())?;
    let Some(present) = copied else {
        return Ok(None);
    };
    let bytes = device.read_back(staging, step)?;
    let (kept_width, kept_height) = region.kept_size(step);
    let pixels = Frame { width: kept_width, height: kept_height, format: opened.format, bytes };
    Ok(Some(RegionFrame { present, pixels }))
}

/// What one look at the header found.
enum Look<T> {
    Frame(T),
    /// No new frame, or the core held the shared texture past the wait.
    NoFrame,
    /// The shared texture failed to open, as in the gap between the core dropping the old texture
    /// and sharing the new one.
    OpenFailed(GpuError),
}

/// The wait after a look that found no frame: `frame_poll` when the look found no new frame, and
/// [`POLL`] when the shared texture failed to open, since each retry of the open calls the GPU.
const fn retry_wait(open_failure: Option<&GpuError>, frame_poll: Duration) -> Duration {
    match open_failure {
        Some(_) => POLL,
        None => frame_poll,
    }
}

/// The device in `slot` when it is on the adapter `adapter_luid`, else a new one there. A new device
/// first drops `opened`, which belongs to the old one.
fn device_on<'slot>(
    slot: &'slot mut Option<Device>,
    opened: &mut Option<Opened>,
    adapter_luid: AdapterLuid,
) -> Result<&'slot Device, GpuError> {
    let device = match slot.take() {
        Some(device) if device.adapter_luid() == adapter_luid => device,
        stale => {
            *opened = None;
            // The old device is released before a device on the new adapter is created.
            drop(stale);
            Device::create(adapter_luid)?
        }
    };
    Ok(slot.insert(device))
}

/// The shared texture, the generation it was opened at, the texture the CPU reads it through, the
/// texture the CPU reads the last region through, and the frame counter of the frame last returned
/// from it.
struct Opened {
    generation: u32,
    shared: SharedTexture,
    staging: Staging,
    region: Option<Staging>,
    format: PixelFormat,
    consumed: u32,
}

impl Opened {
    /// The texture in `slot` when it was opened at the generation `header` holds, else the texture
    /// `name` opened again, with the header's frame counter counted as returned. The inner error is
    /// a failed open, which leaves `slot` empty for the next look to retry.
    fn open<'slot>(
        slot: &'slot mut Option<Self>,
        device: &Device,
        name: &HSTRING,
        header: &ViewerHeader,
    ) -> Result<Result<&'slot mut Self, GpuError>, ReadError> {
        let generation = header.generation();
        let opened = match slot.take() {
            Some(opened) if opened.generation == generation => opened,
            stale => {
                drop(stale);
                let shared = match device.open(name) {
                    Ok(shared) => shared,
                    Err(error) => return Ok(Err(error)),
                };
                let dxgi_format = shared.desc().Format;
                let Some(format) = PixelFormat::from_dxgi(dxgi_format) else {
                    return Err(ReadError::UnsupportedFormat(dxgi_format.0.cast_unsigned()));
                };
                let staging = device.create_staging(&shared, shared.size())?;
                Self { generation, shared, staging, region: None, format, consumed: header.frame() }
            }
        };
        Ok(Ok(slot.insert(opened)))
    }
}

#[cfg(test)]
mod tests;
