//! The depth steps of each session frame in both views: the parallel share write of `share.rs`
//! while the screen quad shows a game frame, then the depth map read of `depth.rs`, which the
//! pointer's trace takes with the head that the share write located on the screen quad and with
//! the eye views in the projection picture.

use std::time::Instant;

use ipc_layout::viewer_header::depth_map;
use xr::error::XrError;
use xr::head::Head;
use xr::quad::ScreenQuad;

use super::depth::{DepthWatch, FrameDepth};
use super::picture::Link;
use super::share::ShareWriter;

/// The share writer and the depth map watch.
pub(crate) struct QuadDepth {
    share_writer: ShareWriter,
    depth_watch: DepthWatch,
}

impl QuadDepth {
    /// The steps at the player's eye distance `ipd_m`, with no map read yet.
    pub(crate) fn new(ipd_m: f32) -> Self {
        Self {
            share_writer: ShareWriter::new(ipd_m),
            depth_watch: DepthWatch::new(depth_map::MAPPING_NAME),
        }
    }

    /// Writes the parallel share of `quad`, the screen quad of a game frame, at the head that
    /// `locate` gives into the viewer header of `link`, then reads the depth map at `now`. With no
    /// quad, `locate` is not called, nothing is written and the frame has no head.
    ///
    /// # Errors
    ///
    /// Returns the [`XrError`] of a head read that lost the session.
    pub(crate) fn frame(
        &mut self,
        quad: Option<ScreenQuad>,
        locate: impl FnOnce() -> Result<Option<Head>, XrError>,
        link: &mut Link,
        now: Instant,
    ) -> Result<FrameDepth<'_>, XrError> {
        let located = quad.map(|shown| (shown, locate()));
        if let Some((shown, read)) = &located {
            self.share_writer.write(read, *shown, link)?;
        }
        // The writer returned a read that lost the session and warned of every other failed one.
        let head = located.and_then(|(_, read)| read.unwrap_or_default());
        Ok(FrameDepth { map: self.depth_watch.read(now), head })
    }
}

#[cfg(test)]
mod tests;
