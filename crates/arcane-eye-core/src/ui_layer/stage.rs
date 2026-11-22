//! The UI layer's hold of the composite stage's UI enable byte: the stage of the pipeline's special
//! view, who drew a frame's UI as the byte shows it, the write of 0 and the give-back of the game's
//! value.

use diag::rate::RateLimit;
use diag::warn_every;
use game_bindings::composite_stage::CompositeStage;
use game_bindings::pipeline::Pipeline;
use game_bindings::render_view::RenderView;
use game_mem::{GameAddr, try_read};

use super::LOG_PERIOD;
use super::decide::Seen;
use crate::raw::composite_stage::stage_writes;

/// The UI enable byte that keeps the game's composite job from drawing the UI texture.
const UI_OFF: u8 = 0;

/// A composite stage whose UI enable byte the core holds at 0, and the value the game had there.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Cleared {
    stage: CompositeStage,
    game_value: u8,
}

/// A frame's composite stage, the value its UI enable byte holds and who drew the frame's UI.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct StageRead {
    stage: CompositeStage,
    enabled: u8,
    pub(super) seen: Seen,
}

/// The UI enable byte's render thread state, held in the UI layer.
pub(super) struct EnableByte {
    /// The global that holds the render pipeline.
    pipeline: GameAddr,
    /// The composite stage's vtable.
    vtable: GameAddr,
    cleared: Option<Cleared>,
    write_log: RateLimit,
}

impl EnableByte {
    pub(super) const fn new(pipeline: GameAddr, vtable: GameAddr) -> Self {
        Self { pipeline, vtable, cleared: None, write_log: RateLimit::new() }
    }

    pub(super) const fn vtable(&self) -> GameAddr {
        self.vtable
    }

    /// This frame's stage, its byte and who drew the frame's UI, or `None` when the pipeline holds
    /// no stage or the byte cannot be read. Forgets a held 0 that the game overwrote or that
    /// belongs to a stage the pipeline no longer holds.
    pub(super) fn read(&mut self) -> Option<StageRead> {
        let stage = self.stage()?;
        let enabled = stage.ui_enabled()?;
        let held = self.cleared.filter(|cleared| cleared.stage == stage && enabled == UI_OFF);
        self.cleared = held;
        let seen = match (enabled, held) {
            (UI_OFF, Some(_)) => Seen::LeftToCore,
            (UI_OFF, None) => Seen::OffByGame,
            _ => Seen::DrawnByGame,
        };
        Some(StageRead { stage, enabled, seen })
    }

    /// Writes 0 to the byte that `read` found, so the game's composite job of the next frame draws
    /// no UI, and holds the game's value for the give-back.
    pub(super) fn clear(&mut self, read: StageRead) {
        self.write(read.stage, UI_OFF);
        self.cleared = Some(Cleared { stage: read.stage, game_value: read.enabled });
    }

    /// Gives the byte the game's value back when the core still holds it at 0 in the stage the
    /// pipeline holds now.
    pub(super) fn give_back(&mut self) {
        let Some(cleared) = self.cleared.take() else { return };
        if self.stage() == Some(cleared.stage) && cleared.stage.ui_enabled() == Some(UI_OFF) {
            self.write(cleared.stage, cleared.game_value);
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
}
