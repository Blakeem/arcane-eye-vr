//! The UI layer's install: the pipeline global and the composite stage's vtable resolved in the
//! game exe's code, with the outcome in the health registry and the log.

use diag::health::{Feature, Health};
use game_bindings::signatures::{G_PIPELINE, VT_COMPOSITE_STAGE};
use settings::dev::SecondView;
use tracing::{info, warn};

use super::UiLayer;
use crate::config::Config;
use crate::install::{GameCode, LocateError};

/// Why the UI layer is not installed.
#[derive(Debug, thiserror::Error)]
enum UiLayerError {
    #[error("the Present call-site hook, which runs the UI layer each frame, is not running")]
    NoFrameEnd,
    #[error(transparent)]
    Locate(#[from] LocateError),
}

/// Resolves the pipeline global and the composite stage's vtable in `code`, the game exe's code,
/// when `config` turns `second_view` on and the Present call-site hook is running, and reports the
/// outcome in the health registry and the log. A failure leaves the feature off and the game
/// unchanged.
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
