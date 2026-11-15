//! The frame handler's link to the debug viewer.
//!
//! On the first frame with devices it creates the viewer header. On each later frame the publisher
//! copies the back buffer into the shared texture while a viewer runs, or the game source's texture
//! when the frame handler holds one and it yields a texture. The texture reference it takes drops
//! before the frame handler returns.

use std::time::{Duration, Instant};

use diag::rate::{Once, RateLimit};
use diag::{error_every, once, warn_every};
use gfx::publish::{Outcome, Publisher, ViewNames};
use tracing::Level;
use windows::Win32::Graphics::Dxgi::IDXGISwapChain;

use crate::raw::{self, Devices};
use crate::viewer_source::GameSource;

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
}

/// The viewer link's render thread state, held in the frame state.
pub(crate) struct ViewerLink {
    link: Link,
    logs: LinkLogs,
}

impl Default for ViewerLink {
    fn default() -> Self {
        Self::new(ViewNames::default())
    }
}

impl ViewerLink {
    pub(crate) fn new(names: ViewNames) -> Self {
        Self { link: Link::Pending(names), logs: LinkLogs::default() }
    }

    /// Runs the viewer work for one frame of `swapchain`, whose device and immediate context are
    /// `devices`. The viewer gets the texture of `game`, and the back buffer for `None` or in a
    /// frame where `game` yields no texture.
    pub(crate) fn on_frame(
        &mut self,
        swapchain: &IDXGISwapChain,
        devices: &Devices,
        game: Option<&mut GameSource>,
    ) {
        let Self { link, logs } = self;
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
        let outcome = publisher.on_frame(Instant::now(), device, context, source);
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
