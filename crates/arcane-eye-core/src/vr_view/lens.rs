//! The lens in use: the lens a frame's proxies get in place of the game's over a centered
//! perspective projection of the head controller.
//!
//! A screen frame keeps the game's lens. A world frame takes the viewing side's lens wish while it
//! lies within the wish's bounds, else the lens of the dev flags `vr.lens_fov_y_deg` and
//! `vr.lens_aspect` while the first is not 0, else the game's lens.

use ipc_layout::viewer_header::lens_wish::LensWish;
use settings::dev::lens::{LensAspect, LensFovYDeg};
use stereo_core::lens::Lens;

use super::ViewKind;

/// The lens in use and where it comes from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum LensInUse {
    /// The viewing side's lens wish.
    Wish(Lens),
    /// The lens of the dev flags `vr.lens_fov_y_deg` and `vr.lens_aspect`.
    Flag(Lens),
    /// The game's own lens, which the proxies keep.
    Game,
}

impl LensInUse {
    /// The lens in use of a frame of the view kind `kind` while the viewing side's lens wish is
    /// `wish`, `None` for no wish or a viewing side that does not beat, and the dev flags are
    /// `fov_y_deg` and `aspect`.
    pub(crate) fn of(
        kind: ViewKind,
        wish: Option<LensWish>,
        fov_y_deg: LensFovYDeg,
        aspect: LensAspect,
    ) -> Self {
        if kind == ViewKind::Screen {
            return Self::Game;
        }
        if let Some(lens) = wish.and_then(wished) {
            return Self::Wish(lens);
        }
        match fov_y_deg.radians().and_then(|fov_y_rad| Lens::new(fov_y_rad, aspect.aspect())) {
            Some(lens) => Self::Flag(lens),
            None => Self::Game,
        }
    }

    /// The lens the proxies get, or `None` for the game's lens.
    pub(crate) const fn lens(self) -> Option<Lens> {
        match self {
            Self::Wish(lens) | Self::Flag(lens) => Some(lens),
            Self::Game => None,
        }
    }

    /// The name the `vr view` line gives the source.
    pub(crate) const fn source(self) -> &'static str {
        match self {
            Self::Wish(_) => "wish",
            Self::Flag(_) => "flag",
            Self::Game => "game",
        }
    }

    /// The vertical field of view in degrees and the aspect the `vr view` line shows, 0 and 0 for
    /// the game's lens.
    pub(crate) fn line_values(self) -> (f32, f32) {
        self.lens().map_or((0.0, 0.0), |lens| (lens.fov_y_rad().to_degrees(), lens.aspect()))
    }
}

/// The lens of `wish`, or `None` for a wish outside [`LensWish::is_within_bounds`].
fn wished(wish: LensWish) -> Option<Lens> {
    if wish.is_within_bounds() { Lens::new(wish.fov_y_rad, wish.aspect) } else { None }
}

#[cfg(test)]
mod tests;
