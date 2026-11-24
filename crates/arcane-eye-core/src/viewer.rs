//! The frame handler's link to the debug viewer.
//!
//! On the first frame with devices it creates the viewer header. On each later frame the publisher
//! copies the back buffer into the shared texture while a viewer runs, or the game source's texture
//! when the frame handler holds one and it yields a texture, with the frame's tag. The texture
//! reference it takes drops before the frame handler returns. Once the header exists, the link reads
//! the head pose, the screen's parallel share and the view wish the viewing side wrote into it while
//! the viewing side's heartbeat runs. While it runs, the link also gives the frame handler the depth
//! map's section, which it makes at the first such frame, for each frame's depth map. The UI layer
//! writes that map, or the frame handler writes a map of no cell in a core with no UI layer.

use std::time::{Duration, Instant};

use diag::rate::{Once, RateLimit};
use diag::{error_every, once, warn_every};
use gfx::depth_map::MapSection;
use gfx::publish::{Outcome, Present, Publisher, ViewNames};
use ipc_layout::viewer_header::pose::{FrameTag, HeadSample};
use ipc_layout::viewer_header::screen::ParallelShare;
use ipc_layout::viewer_header::view_wish::ViewWish;
use tracing::Level;
use windows::Win32::Graphics::Dxgi::IDXGISwapChain;

use crate::game_source::GameSource;
use crate::raw::{self, Devices};
use crate::ui_layer::depth_map::MapSink;

/// The shortest time between two lines of one rate-limited call site.
const LOG_PERIOD: Duration = Duration::from_secs(10);

/// The publisher's state across frames.
enum Link {
    /// The header does not exist yet.
    Pending(ViewNames),
    Ready(Publisher),
    /// The header could not be created, so the core publishes nothing for its life.
    Failed,
}

/// The rate-limit state of each log call site of the viewer link.
#[derive(Default)]
struct LinkLogs {
    setup_failed: Once,
    first_publish: Once,
    source_fault: RateLimit,
    abandoned: RateLimit,
    publish_failed: RateLimit,
    map_failed: Once,
}

/// The depth map's section across frames, made at the first frame that publishes a map.
enum MapLink {
    /// The section does not exist yet, and its name.
    Pending(String),
    Ready(MapSection),
    /// The section could not be created, so the core publishes no depth map for its life.
    Failed,
}

/// The viewer link's render thread state, held in the frame state.
pub(crate) struct ViewerLink {
    link: Link,
    map: MapLink,
    logs: LinkLogs,
}

impl Default for ViewerLink {
    fn default() -> Self {
        Self::new(ViewNames::default())
    }
}

impl ViewerLink {
    pub(crate) fn new(names: ViewNames) -> Self {
        let map = MapLink::Pending(names.depth_map.clone());
        Self { link: Link::Pending(names), map, logs: LinkLogs::default() }
    }

    /// Where the depth map of the frame at `now` goes: the depth map's section and the present
    /// number the frame gets, while the viewing side's heartbeat runs. `None` before the header
    /// exists, after its creation failed, while no viewing side beats, or after the section's
    /// creation failed, which one warning names.
    pub(crate) fn map_sink(&mut self, now: Instant) -> Option<MapSink<'_>> {
        let Link::Ready(publisher) = &mut self.link else { return None };
        let present = publisher.next_present(now)?;
        if let MapLink::Pending(name) = &self.map {
            self.map = match MapSection::create(name) {
                Ok(section) => MapLink::Ready(section),
                Err(error) => {
                    once!(
                        self.logs.map_failed,
                        Level::WARN,
                        error = %error,
                        "the depth map's section could not be created, so the viewer gets no depth map"
                    );
                    MapLink::Failed
                }
            };
        }
        match &self.map {
            MapLink::Ready(section) => Some(MapSink { section, present }),
            MapLink::Pending(_) | MapLink::Failed => None,
        }
    }

    /// The head pose the viewing side wrote last into the header, read at `now`, or `None` before the
    /// header exists, after its creation failed, or while [`Publisher::head_pose`] reads none, such
    /// as after the viewing side stopped beating.
    pub(crate) fn head_pose(&mut self, now: Instant) -> Option<HeadSample> {
        match &mut self.link {
            Link::Ready(publisher) => publisher.head_pose(now),
            Link::Pending(_) | Link::Failed => None,
        }
    }

    /// The parallel share of the screen the viewing side wrote last into the header, read at `now`,
    /// or `None` before the header exists, after its creation failed, or while
    /// [`Publisher::parallel_share`] reads none, such as after the viewing side stopped beating.
    pub(crate) fn parallel_share(&mut self, now: Instant) -> Option<ParallelShare> {
        match &mut self.link {
            Link::Ready(publisher) => publisher.parallel_share(now),
            Link::Pending(_) | Link::Failed => None,
        }
    }

    /// The view wish the viewing side wrote last into the header, read at `now` by
    /// [`Publisher::view_wish`], which gives [`ViewWish::World`] after the viewing side stopped
    /// beating. It is also `World` before the header exists and after its creation failed.
    pub(crate) fn view_wish(&mut self, now: Instant) -> ViewWish {
        match &mut self.link {
            Link::Ready(publisher) => publisher.view_wish(now),
            Link::Pending(_) | Link::Failed => ViewWish::World,
        }
    }

    /// Runs the viewer work for one frame of `swapchain`, whose device and immediate context are
    /// `devices`, tagged with `tag`. The viewer gets the texture of `game`, and the back buffer for
    /// `None` or in a frame where `game` yields no texture.
    pub(crate) fn on_frame(
        &mut self,
        swapchain: &IDXGISwapChain,
        devices: &Devices,
        game: Option<&mut GameSource>,
        tag: FrameTag,
    ) {
        let Self { link, logs, .. } = self;
        let publisher = match link {
            Link::Ready(publisher) => publisher,
            Link::Failed => return,
            Link::Pending(names) => {
                *link = match Publisher::create(names) {
                    Ok(publisher) => Link::Ready(publisher),
                    Err(error) => {
                        once!(
                            logs.setup_failed,
                            Level::ERROR,
                            error = %error,
                            "the debug viewer header could not be created, so the viewer gets no frames"
                        );
                        Link::Failed
                    }
                };
                return;
            }
        };
        let (device, context) = devices;
        let source = || match game.map(GameSource::texture) {
            Some(Ok(texture)) => Ok(texture),
            Some(Err(fault)) => {
                warn_every!(
                    logs.source_fault,
                    LOG_PERIOD,
                    fault = %fault,
                    "the viewer source gave no game texture, so the viewer gets the back buffer"
                );
                raw::back_buffer_texture(swapchain)
            }
            None => raw::back_buffer_texture(swapchain),
        };
        let present = Present { now: Instant::now(), tag };
        let outcome = publisher.on_frame(present, device, context, source);
        match outcome {
            Ok(Outcome::Published) => {
                once!(logs.first_publish, Level::INFO, "the debug viewer receives frames");
            }
            Ok(Outcome::Abandoned) => warn_every!(
                logs.abandoned,
                LOG_PERIOD,
                "a debug viewer exited while holding the shared texture, so the texture was recreated"
            ),
            Ok(Outcome::NoViewer | Outcome::ViewerBusy | Outcome::Unshareable) => {}
            Err(error) => error_every!(
                logs.publish_failed,
                LOG_PERIOD,
                error = %error,
                "publishing to the debug viewer failed"
            ),
        }
    }
}

#[cfg(test)]
mod tests;
