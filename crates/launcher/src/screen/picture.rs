//! The game's side of the headset session: the viewer link's header with the head pose write, the
//! screen write and the wishes write, the shared texture opened at its generation, and the
//! private copy of the newest published frame that the eye draws read, with the frame's tag.
//!
//! The copy holds the keyed mutex for one copy of the entire frame and the read of the frame's tag,
//! and waits on it at most as long as [`Device::copy_region`] does. A session frame with no new
//! published frame takes no lock and shows the private copy again.

use std::time::Duration;

use diag::rate::RateLimit;
use diag::warn_every;
use gfx::region::{RegionDraw, RegionError};
use ipc_layout::viewer_header::lens_wish::LensWish;
use ipc_layout::viewer_header::pose::{FrameTag, HeadPose};
use ipc_layout::viewer_header::screen::ParallelShare;
use ipc_layout::viewer_header::{AdapterLuid, TextureInfo};
use tracing::info;
use viewer_client::frame::PixelFormat;
use viewer_client::header::{
    HeaderReadError, Names, read_header, write_head_pose, write_horizon_wish, write_lens_wish,
    write_scale_wish, write_screen, write_view_wish,
};
use viewer_client::region::Region;
use viewer_client::{Device, GpuError, HeaderView, SharedTexture};
use windows::Win32::Graphics::Direct3D11::{ID3D11ShaderResourceView, ID3D11Texture2D};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT;
use windows::core::HSTRING;
use xr::format::Sampling;

use super::menu::Wishes;

/// The shortest time between two log lines of a failed frame read.
const LOG_PERIOD: Duration = Duration::from_secs(5);

/// Why the screen cannot show the game's frames for as long as the game runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub(crate) enum Unshowable {
    #[error("the game draws on another graphics adapter than the headset's")]
    OtherAdapter,
    #[error("the game's frames have the DXGI format {0}, which the screen does not show")]
    UnsupportedFormat(u32),
}

/// Why the newest published frame could not be read this time.
#[derive(Debug, thiserror::Error)]
enum LinkError {
    #[error(transparent)]
    Header(#[from] HeaderReadError),
    #[error(transparent)]
    Unshowable(#[from] Unshowable),
    #[error(transparent)]
    Gpu(#[from] GpuError),
    #[error(transparent)]
    Draw(#[from] RegionError),
}

/// Whether the screen can show the frames of `texture` on a device on the adapter `luid`.
fn showable(texture: &TextureInfo, luid: AdapterLuid) -> Result<(), Unshowable> {
    if texture.adapter_luid != luid {
        return Err(Unshowable::OtherAdapter);
    }
    if PixelFormat::from_dxgi(DXGI_FORMAT(texture.format.cast_signed())).is_none() {
        return Err(Unshowable::UnsupportedFormat(texture.format));
    }
    Ok(())
}

/// The shared texture of one generation, its private copy and the view the eye draws read.
pub(crate) struct Picture {
    view: ID3D11ShaderResourceView,
    copy: ID3D11Texture2D,
    shared: SharedTexture,
    generation: u32,
    sampling: Sampling,
    /// The header's frame counter when the copy last took a frame, or `None` before the first.
    copied_frame: Option<u32>,
    /// The tag of the frame the copy holds, [`FrameTag::NONE`] before the first.
    tag: FrameTag,
}

impl Picture {
    /// Opens the shared texture `name` of `generation` on `device` and creates its private copy.
    fn open(
        device: &Device,
        draw: &RegionDraw,
        name: &HSTRING,
        generation: u32,
    ) -> Result<Self, LinkError> {
        let shared = device.open(name)?;
        let format = shared.desc().Format;
        let copy = device.create_private_copy(&shared)?;
        let view = draw.source_view(&copy)?;
        let (width, height) = shared.size();
        info!(generation, width, height, format = format.0, "game frames opened");
        Ok(Self {
            view,
            copy,
            shared,
            generation,
            sampling: Sampling::of(format),
            copied_frame: None,
            tag: FrameTag::NONE,
        })
    }

    /// The size of the entire published frame, `(width, height)`.
    pub(crate) const fn size(&self) -> (u32, u32) {
        self.shared.size()
    }

    /// The tag of the frame the private copy holds, read while the copy held the keyed mutex.
    pub(crate) const fn tag(&self) -> FrameTag {
        self.tag
    }

    /// A view of the private copy in the published frame's format.
    pub(crate) const fn view(&self) -> &ID3D11ShaderResourceView {
        &self.view
    }

    /// What a shader read of the private copy returns.
    pub(crate) const fn sampling(&self) -> Sampling {
        self.sampling
    }
}

/// The session's view of the viewer link, which reads the frames and writes the head pose, the
/// screen and the menu's wishes. The heartbeat thread holds its own view of the header.
pub(crate) struct Link {
    picture: Option<Picture>,
    header: Option<HeaderView>,
    names: Names,
    read_log: RateLimit,
}

impl Link {
    pub(crate) const fn new(names: Names) -> Self {
        Self { picture: None, header: None, names, read_log: RateLimit::new() }
    }

    /// Copies the published frame into the picture when the header numbers a frame the picture
    /// lacks, and returns the picture once it holds a frame. A failed read is a rate-limited log
    /// line, and the next call tries again.
    ///
    /// # Errors
    ///
    /// Returns the [`Unshowable`] cause of frames that no later call can show.
    pub(crate) fn refresh(
        &mut self,
        device: &Device,
        draw: &RegionDraw,
    ) -> Result<Option<&Picture>, Unshowable> {
        match self.update(device, draw) {
            Ok(()) => {}
            Err(LinkError::Unshowable(cause)) => return Err(cause),
            Err(error) => {
                warn_every!(
                    self.read_log,
                    LOG_PERIOD,
                    error = %error,
                    "the game's frame could not be read"
                );
            }
        }
        Ok(self.picture.as_ref().filter(|picture| picture.copied_frame.is_some()))
    }

    /// Writes `pose` and `eye_distance_m` as the head pose into the viewer header, and returns the
    /// sequence number the write ended with, or `None` while no core created the header.
    ///
    /// # Errors
    ///
    /// Returns the [`HeaderReadError`] of a header that cannot be opened or holds another layout.
    pub(crate) fn write_head(
        &mut self,
        pose: HeadPose,
        eye_distance_m: f32,
    ) -> Result<Option<u32>, HeaderReadError> {
        let Some(view) = self.header_view()? else { return Ok(None) };
        Ok(Some(write_head_pose(view, pose, eye_distance_m)?))
    }

    /// Writes `share`, the parallel share of the screen quad, into the viewer header, and writes
    /// nothing while no core created the header.
    ///
    /// # Errors
    ///
    /// Returns the [`HeaderReadError`] of a header that cannot be opened or holds another layout.
    pub(crate) fn write_screen(&mut self, share: ParallelShare) -> Result<(), HeaderReadError> {
        let Some(view) = self.header_view()? else { return Ok(()) };
        write_screen(view, share)?;
        Ok(())
    }

    /// Writes the view wish, the horizon wish and the scale wish of `wishes`, and `lens` when it
    /// holds a lens wish, into the viewer header, and writes nothing while no core created the
    /// header.
    ///
    /// # Errors
    ///
    /// Returns the [`HeaderReadError`] of a header that cannot be opened or holds another layout.
    pub(crate) fn write_wishes(
        &mut self,
        wishes: Wishes,
        lens: Option<LensWish>,
    ) -> Result<(), HeaderReadError> {
        let Some(view) = self.header_view()? else { return Ok(()) };
        write_view_wish(view, wishes.view)?;
        write_horizon_wish(view, wishes.horizon)?;
        write_scale_wish(view, wishes.scale)?;
        if let Some(lens) = lens {
            write_lens_wish(view, lens)?;
        }
        Ok(())
    }

    /// The viewer header a write goes into, or `None` while no core created it.
    fn header_view(&mut self) -> Result<Option<&HeaderView>, HeaderReadError> {
        match read_header(&mut self.header, &self.names.mapping) {
            Ok((view, _header)) => Ok(Some(view)),
            Err(HeaderReadError::Missing) => Ok(None),
            Err(error) => Err(error),
        }
    }

    fn update(&mut self, device: &Device, draw: &RegionDraw) -> Result<(), LinkError> {
        let (view, header) = match read_header(&mut self.header, &self.names.mapping) {
            Ok(read) => read,
            Err(HeaderReadError::Missing) => return Ok(()),
            Err(error) => return Err(error.into()),
        };
        let texture = header.texture();
        // The core shares no texture while it replaces one, so the picture keeps its last frame.
        if texture.width == 0 {
            return Ok(());
        }
        showable(&texture, device.adapter_luid())?;
        let picture = match self.picture.take() {
            Some(picture) if picture.generation == header.generation() => picture,
            stale => {
                drop(stale);
                Picture::open(device, draw, &self.names.texture, header.generation())?
            }
        };
        let picture = self.picture.insert(picture);
        let frame = header.frame();
        if picture.copied_frame == Some(frame) {
            return Ok(());
        }
        let (width, height) = picture.shared.size();
        let entire = Region::entire(width, height);
        match device.copy_region(&picture.shared, &picture.copy, entire, || view.read_frame_tag()) {
            Ok(Some(tag)) => (picture.copied_frame, picture.tag) = (Some(frame), tag),
            Ok(None) => {}
            // An abandoned mutex or a failed copy reopens the shared texture at the next call.
            Err(error) => {
                self.picture = None;
                return Err(error.into());
            }
        }
        Ok(())
    }
}
