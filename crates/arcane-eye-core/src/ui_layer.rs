//! The UI layer feature: every frame the game presents is a half side by side pair, and the core
//! draws the game's UI texture into each half of the back buffer, in place of the game's own draw
//! across the entire frame.
//!
//! ```text
//! PRESENT CALL SITE, render thread, before the debug viewer's copy
//!   [composite stage]   found each frame: pipeline global, special view, stage array, vtable
//!   [UI enable byte]    read. The composite job of this frame read the same value, since the core
//!                       writes the byte only here
//!   [decide]            what the core draws into this frame and what it writes for the next
//!   [pair]              a flat frame's entire picture into each half of the back buffer
//!   [draw]              the UI key's texture over each half of the back buffer
//!   [write]             0 while the feature runs, the game's value when it stops
//! ```
//!
//! The feature has no hook of its own. The frame handler runs it once per frame. The engine renders
//! flat frames before the eye view is ready, such as the start-up video and a level load. The game's
//! UI stage fills the UI texture whether or not the byte is set, the texture table holds the UI
//! texture in the start-up video and the loading picture, and the texture is premultiplied (all
//! measured in the game). A draw that fails switches the feature off. A feature that is off
//! presents flat frames flat and gives the byte the game's value back, so the game draws its UI
//! again.

mod decide;

use std::mem;
use std::time::Duration;

use diag::health::{Feature, Health};
use diag::rate::RateLimit;
use diag::warn_every;
use game_bindings::composite_stage::CompositeStage;
use game_bindings::pipeline::Pipeline;
use game_bindings::render_view::RenderView;
use game_bindings::signatures::{G_PIPELINE, VT_COMPOSITE_STAGE};
use game_bindings::texture_table::TextureKey;
use game_mem::{GameAddr, try_read};
use gfx::layer::{LayerDraw, LayerError};
use settings::dev::SecondView;
use tracing::{info, warn};
use windows::Win32::Graphics::Dxgi::IDXGISwapChain;
use windows::core::HRESULT;

use self::decide::{Draw, Picture, Seen, Write, halves, step};
use crate::config::Config;
use crate::eye_view::Layout;
use crate::game_source::{GameSource, SourceFault};
use crate::install::{GameCode, LocateError};
use crate::present;
use crate::raw::composite_stage::stage_writes;
use crate::raw::{self, BackBuffer, Devices};

/// The shortest time between two warnings of one call site.
const LOG_PERIOD: Duration = Duration::from_secs(10);

/// The UI enable byte that keeps the game's composite job from drawing the UI texture.
const UI_OFF: u8 = 0;

/// Why the UI layer is not installed.
#[derive(Debug, thiserror::Error)]
enum UiLayerError {
    #[error("the Present call-site hook, which runs the UI layer each frame, is not running")]
    NoFrameEnd,
    #[error(transparent)]
    Locate(#[from] LocateError),
}

/// Why the pair or the UI texture was not drawn into a frame.
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

/// The back buffer of the frame at the Present call site.
pub(crate) struct Target<'frame> {
    pub(crate) swapchain: &'frame IDXGISwapChain,
    pub(crate) devices: &'frame Devices,
    pub(crate) size: BackBuffer,
}

/// A composite stage whose UI enable byte the core holds at 0, and the value the game had there.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Cleared {
    stage: CompositeStage,
    game_value: u8,
}

/// The feature's render thread state, held in the frame state.
pub(crate) struct UiLayer {
    /// The global that holds the render pipeline.
    pipeline: GameAddr,
    /// The composite stage's vtable.
    vtable: GameAddr,
    source: GameSource,
    /// The layer draw of the device that the last drawn frame had, dropped when the feature stops.
    layer: Option<LayerDraw>,
    /// The layout of the frame at the next Present call site.
    next: Layout,
    cleared: Option<Cleared>,
    stage_log: RateLimit,
    write_log: RateLimit,
}

/// Resolves the pipeline global and the composite stage's vtable in `code`, the game exe's code, when
/// `config` turns `second_view` on and the Present call-site hook is running, and reports the outcome
/// in the health registry and the log. A failure leaves the feature off and the game unchanged.
pub(crate) fn install(health: &Health, code: &GameCode, config: Config) -> Option<UiLayer> {
    if config.dev.stereo.second_view == SecondView::Off {
        info!(second_view = false, "ui layer not installed");
        return None;
    }
    match locate(health, code) {
        Ok(layer) => {
            health.install_ok(Feature::UiLayer);
            info!(vtable = format_args!("{:#x}", layer.vtable.get()), "ui layer installed");
            Some(layer)
        }
        Err(error) => {
            health.install_failed(Feature::UiLayer);
            warn!(error = %error, "ui layer not installed");
            None
        }
    }
}

fn locate(health: &Health, code: &GameCode) -> Result<UiLayer, UiLayerError> {
    if !health.is_running(Feature::PresentHook) {
        return Err(UiLayerError::NoFrameEnd);
    }
    Ok(UiLayer::new(code.locate(&G_PIPELINE)?, code.locate(&VT_COMPOSITE_STAGE)?))
}

impl UiLayer {
    /// A UI layer that reads the pipeline from the global `pipeline` and finds the composite stage by
    /// `vtable`.
    pub(crate) fn new(pipeline: GameAddr, vtable: GameAddr) -> Self {
        Self {
            pipeline,
            vtable,
            source: GameSource::new(TextureKey::UI, pipeline),
            layer: None,
            next: Layout::MainOnly,
            cleared: None,
            stage_log: RateLimit::default(),
            write_log: RateLimit::default(),
        }
    }

    /// Runs the feature for the frame whose back buffer is `target`. `next` is the layout of the
    /// frame after it, which the eye view feature read at this Present call site. The work runs
    /// inside its own `catch_unwind`, so a panic latches `ui-layer` off and leaves the frame handler
    /// running.
    pub(crate) fn on_frame(&mut self, health: &Health, next: Layout, target: &Target<'_>) {
        let layout = mem::replace(&mut self.next, next);
        present::guarded(health, Feature::UiLayer, || {
            if let Err(fault) = self.run(layout, target) {
                // The failed check switches the feature off, so this line appears once.
                warn!(
                    fault = %fault,
                    "ui layer draw failed, so flat frames stay flat and the game draws its UI again"
                );
                health.check_failed(Feature::UiLayer);
            }
        });
        if !health.is_running(Feature::UiLayer) {
            self.release();
        }
    }

    /// One frame of `layout`: reads the composite stage's UI enable byte, draws a flat frame's
    /// entire picture into each half, draws the UI texture into each half when the game's composite
    /// job left it out, and writes the byte for the next frame.
    fn run(&mut self, layout: Layout, target: &Target<'_>) -> Result<(), DrawFault> {
        let picture = match layout {
            Layout::SideBySide { .. } => Picture::SideBySide,
            Layout::MainOnly => Picture::Flat,
        };
        let read = self.stage().and_then(|stage| Some((stage, stage.ui_enabled()?)));
        let seen = read.map(|(stage, enabled)| self.see(stage, enabled));
        if seen.is_none() && picture == Picture::SideBySide {
            warn_every!(
                self.stage_log,
                LOG_PERIOD,
                "ui layer found no composite stage, so the game draws its UI"
            );
        }
        let step = step(picture, seen);
        self.draw(step.draw, target)?;
        if let (Write::Clear, Some((stage, enabled))) = (step.write, read) {
            self.write(stage, UI_OFF);
            self.cleared = Some(Cleared { stage, game_value: enabled });
        }
        Ok(())
    }

    /// Who drew the UI of this frame, from the byte `enabled` of `stage`. Forgets a held 0 that the
    /// game overwrote or that belongs to a stage the pipeline no longer holds.
    fn see(&mut self, stage: CompositeStage, enabled: u8) -> Seen {
        let held = self.cleared.filter(|cleared| cleared.stage == stage && enabled == UI_OFF);
        self.cleared = held;
        match (enabled, held) {
            (UI_OFF, Some(_)) => Seen::LeftToCore,
            (UI_OFF, None) => Seen::OffByGame,
            _ => Seen::DrawnByGame,
        }
    }

    /// The composite stage of the pipeline's special view, or `None` when a read fails or the view
    /// holds none.
    fn stage(&self) -> Option<CompositeStage> {
        let pipeline = try_read::<usize>(self.pipeline)?;
        let views = Pipeline::new(GameAddr::new(pipeline)).views()?;
        if views.special.get() == 0 {
            return None;
        }
        CompositeStage::of(RenderView::new(views.special), self.vtable)
    }

    /// Runs `draw` on each half of the back buffer: the pair first, then the UI key's texture of this
    /// frame. The UI texture is read before the pair, so a failed read leaves the frame as the game
    /// drew it.
    fn draw(&mut self, draw: Draw, target: &Target<'_>) -> Result<(), DrawFault> {
        if draw == Draw::Nothing {
            return Ok(());
        }
        let (device, context) = target.devices;
        let BackBuffer { width, height, .. } = target.size;
        let rects = halves(width, height).ok_or(DrawFault::Size { width, height })?;
        let ui = if draw.draws_ui() { Some(self.source.texture()?) } else { None };
        let layer = match self.layer.take().filter(|layer| layer.belongs_to(device)) {
            Some(layer) => layer,
            None => LayerDraw::create(device)?,
        };
        let layer = self.layer.insert(layer);
        let back_buffer = match raw::back_buffer_texture(target.swapchain) {
            Ok(texture) => texture,
            Err(code) => return Err(DrawFault::BackBuffer { code }),
        };
        if draw.makes_pair() {
            layer.draw_copy(context, &back_buffer, &rects)?;
        }
        if let Some(ui) = ui {
            layer.draw(context, &back_buffer, &ui, &rects)?;
        }
        Ok(())
    }

    /// Writes `enabled` to the UI enable byte of `stage`. The game's composite job of the next frame
    /// reads it.
    fn write(&mut self, stage: CompositeStage, enabled: u8) {
        if let Err(error) = CompositeStage::write_ui_enabled(stage_writes(stage), enabled) {
            warn_every!(
                self.write_log,
                LOG_PERIOD,
                error = %error,
                "ui layer write of the UI enable byte failed"
            );
        }
    }

    /// Drops the layer draw, whose copy of the back buffer holds video memory, and gives the UI enable
    /// byte the game's value back when the core still holds it at 0 in the stage the pipeline holds
    /// now.
    fn release(&mut self) {
        self.layer = None;
        let Some(cleared) = self.cleared.take() else { return };
        if self.stage() == Some(cleared.stage) && cleared.stage.ui_enabled() == Some(UI_OFF) {
            self.write(cleared.stage, cleared.game_value);
        }
    }
}

#[cfg(test)]
mod tests;
