//! The UI layer feature: every frame the game presents is a half side by side pair, and the core
//! draws the game's UI texture and the game's mouse cursor into each half of the back buffer, in
//! place of the game's own draw across the entire frame.
//!
//! ```text
//! PRESENT CALL SITE, render thread, before the debug viewer's copy
//!   [composite stage]   found each frame: pipeline global, special view, stage array, vtable, in
//!                       `stage`
//!   [UI enable byte]    read, in `stage`. The composite job of this frame read the same value,
//!                       since the core writes the byte only here
//!   [decide]            the pair, the move and the UI texture the core draws into this frame,
//!                       whether Windows shows its cursor over a drawn cursor, and what the core
//!                       writes for the next frame
//!   [aim]               the drawn cursor's pointer and cursor handle, in `cursor`
//!   [ui depth]          the columns by which a side by side frame's UI moves in front of the
//!                       scene under it, 0 while the UI hides the scene, and the cursor's columns
//!                       at the depth of what is under the pointer, in `ui_depth`. 0 in every
//!                       other frame
//!   [pair]              a flat frame's entire picture into each half of the back buffer
//!   [move]              a side by side frame's right half moved right by the depth offset in
//!                       use, black in the columns it uncovers, in a frame that holds no UI yet
//!   [draw]              the UI key's texture over each half of the back buffer, moved by those
//!                       columns
//!   [cursor]            the game's cursor over each half of the back buffer, in each frame whose
//!                       pointer is in the game window's client area, moved by the cursor's columns
//!   [counter]           the Windows cursor hidden under a drawn cursor as decided, and shown in a
//!                       frame with no drawn cursor, in `cursor`
//!   [write]             0 while the feature runs, the game's value when it stops, in `stage`
//!   [depth map]         how deep each cell of the left half looks after the draw, for a viewing
//!                       program, in `depth_map`
//! ```
//!
//! The feature has no hook of its own. The frame handler runs it once per frame. The engine renders
//! flat frames before the eye view is ready, such as the start-up video and a level load. The game's
//! UI stage fills the UI texture whether or not the byte is set, the texture table holds the UI
//! texture in the start-up video and the loading picture, and the texture is premultiplied (all
//! measured in the game). A failed draw of the pair, the move or the UI texture switches the feature
//! off, which makes the stereo pair flat. A feature that is off presents flat frames flat, leaves the
//! right half where the game rendered it, and gives the byte the game's value back, so the game
//! draws its UI again. It also gives the cursor display counter the game's value back, so Windows
//! shows the cursor over the game window again. The feature stops the same way while the stereo
//! pair is flat after a failure of another feature of the pair (see [`pair`](crate::pair)).

pub(crate) mod cursor;
mod decide;
pub(crate) mod depth_map;
pub(crate) mod depth_offset;
mod stage;
mod target;
pub(crate) mod ui_depth;

use std::mem;
use std::time::Duration;

use diag::health::{Feature, Health};
use diag::rate::RateLimit;
use diag::warn_every;
use game_bindings::signatures::{G_PIPELINE, VT_COMPOSITE_STAGE};
use game_bindings::texture_table::TextureKey;
use game_mem::GameAddr;
use gfx::layer::LayerDraw;
use settings::dev::SecondView;
use stereo_core::eye::EyeScale;
use tracing::{info, warn};

use self::cursor::counter::Marked;
use self::cursor::{Aim, Canvas, Cursor};
use self::decide::{Draw, Picture, Step, Write, crossed, depth_shift, drawn_offset, halves, step};
use self::depth_map::{DepthMap, Drawn, FrameTextures, MapFrame, MapSink, MapSources};
use self::stage::EnableByte;
pub(crate) use self::target::{DrawFault, Target};
use self::ui_depth::{Columns, CursorReport, DepthInput, DepthReport, DrawnUi, UiDepth};
use crate::config::Config;
use crate::eye_view::Layout;
use crate::game_source::GameSource;
use crate::install::{GameCode, LocateError};
use crate::pair::PairState;
use crate::present;
use crate::raw::{self, BackBuffer};

/// The shortest time between two warnings of one call site.
pub(crate) const LOG_PERIOD: Duration = Duration::from_secs(10);

/// Why the UI layer is not installed.
#[derive(Debug, thiserror::Error)]
enum UiLayerError {
    #[error("the Present call-site hook, which runs the UI layer each frame, is not running")]
    NoFrameEnd,
    #[error(transparent)]
    Locate(#[from] LocateError),
}

/// The feature's render thread state, held in the frame state.
pub(crate) struct UiLayer {
    byte: EnableByte,
    source: GameSource,
    /// The layer draw of the device that the last drawn frame had, dropped when the feature stops.
    layer: Option<LayerDraw>,
    /// The layout of the frame at the next Present call site.
    next: Layout,
    cursor: Cursor,
    depth: UiDepth,
    /// The draw of this frame that ran to its end, or `None` for none.
    drawn: Option<Drawn>,
    /// The textures this frame's draw read, until its depth map takes them.
    textures: FrameTextures,
    map: DepthMap,
    stage_log: RateLimit,
    cursor_log: RateLimit,
}

/// Resolves the pipeline global and the composite stage's vtable in `code`, the game exe's code, when
/// `config` turns `second_view` on and the Present call-site hook is running, and reports the outcome
/// in the health registry and the log. A failure leaves the feature off and the game unchanged.
pub(crate) fn install(health: &Health, code: &GameCode, config: Config) -> Option<UiLayer> {
    if config.dev.stereo.second_view == SecondView::Off {
        info!(second_view = false, "ui layer not installed");
        return None;
    }
    match locate(health, code, config) {
        Ok(layer) => {
            health.install_ok(Feature::UiLayer);
            info!(vtable = format_args!("{:#x}", layer.byte.vtable().get()), "ui layer installed");
            Some(layer)
        }
        Err(error) => {
            health.install_failed(Feature::UiLayer);
            warn!(error = %error, "ui layer not installed");
            None
        }
    }
}

fn locate(health: &Health, code: &GameCode, config: Config) -> Result<UiLayer, UiLayerError> {
    if !health.is_running(Feature::PresentHook) {
        return Err(UiLayerError::NoFrameEnd);
    }
    let forward_wait = config.dev.ui.forward_wait_ms.duration();
    Ok(UiLayer::new(code.locate(&G_PIPELINE)?, code.locate(&VT_COMPOSITE_STAGE)?, forward_wait))
}

impl UiLayer {
    /// A UI layer that reads the pipeline from the global `pipeline`, finds the composite stage by
    /// `vtable` and brings the UI forward once a nearer scene held for `forward_wait`.
    pub(crate) fn new(pipeline: GameAddr, vtable: GameAddr, forward_wait: Duration) -> Self {
        Self {
            byte: EnableByte::new(pipeline, vtable),
            source: GameSource::new(TextureKey::UI, pipeline),
            layer: None,
            next: Layout::MainOnly,
            cursor: Cursor::default(),
            depth: UiDepth::new(pipeline, forward_wait),
            drawn: None,
            textures: FrameTextures::default(),
            map: DepthMap::new(),
            stage_log: RateLimit::default(),
            cursor_log: RateLimit::default(),
        }
    }

    /// Runs the feature for the frame whose back buffer is `target`, with the right half of a side
    /// by side frame moved by the depth offset of `input`. `next` is the layout of the frame after
    /// it, which the eye view feature read at this Present call site. The work runs inside its own
    /// `catch_unwind`, so a panic latches `ui-layer` off and leaves the frame handler running. A flat
    /// pair draws and writes nothing but the game's values back.
    pub(crate) fn on_frame(
        &mut self,
        health: &Health,
        next: Layout,
        input: &DepthInput,
        target: &Target<'_>,
    ) {
        let layout = mem::replace(&mut self.next, next);
        self.depth.keep_offset(input.offset);
        self.drawn = None;
        self.textures = FrameTextures::default();
        if PairState::of(health).is_flat() {
            self.release();
            return;
        }
        present::guarded(health, Feature::UiLayer, || {
            if let Err(fault) = self.run(layout, input, target) {
                // The failed check switches the feature off, so this line appears once.
                warn!(
                    fault = %fault,
                    "ui layer draw failed, so the game shows its flat picture and its UI again"
                );
                health.check_failed(Feature::UiLayer);
            }
        });
        if !health.is_running(Feature::UiLayer) {
            self.release();
        }
    }

    /// Shows the Windows cursor in a frame the frame handler skips before the UI layer runs, since
    /// that frame gets no drawn cursor. The layer draw and the cursor's texture stay for the next
    /// frame.
    pub(crate) fn on_skipped_frame(&mut self) {
        self.cursor.show();
    }

    /// Hands the UI depth's eye scale in use to the eye cameras at a Present call.
    pub(crate) fn hand_eye_scale(&mut self) -> EyeScale {
        self.depth.hand_eye_scale()
    }

    pub(crate) fn depth_report(&self) -> DepthReport {
        self.depth.report()
    }

    pub(crate) fn cursor_report(&self) -> CursorReport {
        self.depth.cursor_report()
    }

    /// One frame of `layout`: reads the composite stage's UI enable byte, draws a flat frame's
    /// entire picture into each half, moves a side by side frame's right half by the depth offset
    /// of `input`, draws the UI texture into each half when the game's composite job left it out,
    /// draws the cursor into each half, and writes the byte for the next frame.
    fn run(
        &mut self,
        layout: Layout,
        input: &DepthInput,
        target: &Target<'_>,
    ) -> Result<(), DrawFault> {
        let picture = match layout {
            Layout::SideBySide { .. } => Picture::SideBySide,
            Layout::MainOnly => Picture::Flat,
        };
        let shift = depth_shift(target.size.width, target.size.height, input.offset);
        let read = self.byte.read();
        let seen = read.map(|read| read.seen);
        if seen.is_none() && picture == Picture::SideBySide {
            warn_every!(
                self.stage_log,
                LOG_PERIOD,
                "ui layer found no composite stage, so the game draws its UI"
            );
        }
        let step = step(picture, seen, shift);
        if !step.draw.draws_side_by_side_ui() {
            self.depth.reset();
        }
        let aim = self.cursor.aim(target.window);
        self.draw(step, aim, target, input)?;
        self.drawn = Some(Drawn { draw: step.draw, seen });
        if let (Write::Clear, Some(read)) = (step.write, read) {
            self.byte.clear(read);
        }
        Ok(())
    }

    /// Runs the draw of `step` and then the cursor of `aim` on each half of the back buffer, then
    /// hides the Windows cursor under a drawn cursor when `step` asks, and shows it otherwise. A
    /// fault of a frame that draws only the cursor skips that frame's cursor.
    fn draw(
        &mut self,
        step: Step,
        aim: Option<Aim>,
        target: &Target<'_>,
        input: &DepthInput,
    ) -> Result<(), DrawFault> {
        let draw = step.draw;
        let marked = match (draw, aim) {
            (Draw::Nothing, None) => Marked::Nothing,
            (Draw::Nothing, Some(_)) => {
                // Only the cursor needed the back buffer and the layer draw, and a cursor fault never
                // stops the UI layer.
                self.draw_halves(draw, aim, target, input).unwrap_or_else(|fault| {
                    warn_every!(
                        self.cursor_log,
                        LOG_PERIOD,
                        fault = %fault,
                        "ui layer skipped the cursor of a frame with no other draw"
                    );
                    Marked::Nothing
                })
            }
            _ => self.draw_halves(draw, aim, target, input)?,
        };
        self.cursor.mark(step.windows_cursor, marked);
        Ok(())
    }

    /// Runs `draw` on the back buffer: the pair or the move first, then the UI key's texture of this
    /// frame into each half, then the cursor of `aim` into each half, each moved by its columns the
    /// UI depth gives a side by side frame. The UI texture is read before the pair and the
    /// move, so a failed read leaves the frame as the game drew it. A fault of the cursor's read,
    /// conversion or placed draw skips only the cursor. Keeps the UI texture and the distance
    /// texture the frame read for its depth map. Returns whether the cursor was drawn.
    fn draw_halves(
        &mut self,
        draw: Draw,
        aim: Option<Aim>,
        target: &Target<'_>,
        input: &DepthInput,
    ) -> Result<Marked, DrawFault> {
        let (device, context) = target.devices;
        let BackBuffer { width, height, .. } = target.size;
        let rects = halves(width, height).ok_or(DrawFault::Size { width, height })?;
        let ui = if draw.draws_ui() { Some(self.source.texture()?) } else { None };
        let layer = if let Some(layer) = self.layer.take().filter(|layer| layer.belongs_to(device))
        {
            layer
        } else {
            self.cursor.forget_texture();
            LayerDraw::create(device)?
        };
        let layer = self.layer.insert(layer);
        let columns = match &ui {
            Some(ui) if draw.draws_side_by_side_ui() => {
                let [left, _] = rects;
                let pointer = aim.map(|aim| aim.spot.shares());
                let offset = drawn_offset(draw);
                let drawn = DrawnUi { ui, half_width: left.width, offset, pointer };
                let followed = self.depth.on_frame(target.devices, input, &drawn);
                self.textures.distances = followed.distances;
                followed.columns
            }
            _ => Columns::SURFACE,
        };
        let back_buffer = match raw::back_buffer_texture(target.swapchain) {
            Ok(texture) => texture,
            Err(code) => return Err(DrawFault::BackBuffer { code }),
        };
        if draw.makes_pair() {
            layer.draw_copy(context, &back_buffer, &rects)?;
        }
        if let Some(shift) = draw.shift() {
            layer.draw_copy_placed(context, &back_buffer, shift.source, &[shift.picture])?;
            layer.draw_black(context, &back_buffer, &[shift.uncovered])?;
        }
        if let Some(ui) = &ui {
            layer.draw_placed(context, &back_buffer, ui, &crossed(&rects, columns.ui))?;
        }
        self.textures.ui = ui;
        let Some(aim) = aim else { return Ok(Marked::Nothing) };
        let moves = columns.cursor;
        let canvas = Canvas { context, back_buffer: &back_buffer, halves: &rects, moves };
        Ok(self.cursor.draw(layer, &canvas, aim))
    }

    /// Drops the layer draw, whose copy of the back buffer holds video memory, the cursor's texture,
    /// the UI depth's reads and the depth map's objects, sets the UI depth's values in use to no UI
    /// disparity and the full eye distance, gives the cursor display counter the game's value back,
    /// and gives the UI enable byte the game's value back when the core still holds it at 0 in the
    /// stage the pipeline holds now. The frame handler also runs it after the Present call-site
    /// hook failed.
    pub(crate) fn release(&mut self) {
        self.layer = None;
        self.depth.release();
        self.map.release();
        self.cursor.release();
        self.byte.give_back();
    }

    /// The depth map of the frame the UI layer ran on with `input`, through the devices of `target`
    /// and at its size, into the section `sink` holds. With no sink it forgets the maps that wait
    /// for the GPU.
    ///
    /// The map runs inside the feature's `catch_unwind`, so a panic in it latches `ui-layer` off. A
    /// feature that is off publishes a map of no cell. The frame's textures are dropped here.
    pub(crate) fn publish_map(
        &mut self,
        health: &Health,
        target: &Target<'_>,
        input: &DepthInput,
        sink: Option<MapSink<'_>>,
    ) {
        let textures = mem::take(&mut self.textures);
        let Some(sink) = sink else {
            self.map.forget();
            return;
        };
        let frame = MapFrame {
            pair: PairState::of(health),
            drawn: self.drawn,
            devices: target.devices,
            size: target.size,
            input,
        };
        let sources = MapSources { textures, depth: &mut self.depth };
        let map = &mut self.map;
        present::guarded(health, Feature::UiLayer, || map.publish(&sink, sources, &frame));
        if !health.is_running(Feature::UiLayer) {
            sink.section.write_empty(sink.present);
            self.release();
        }
    }
}

#[cfg(test)]
pub(crate) mod test_support;
#[cfg(test)]
mod tests;
