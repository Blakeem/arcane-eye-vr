//! The pure decisions of the controller models' layer: what a frame's draw gives the session and
//! what a location of the eye views gives the layer.

use stereo_core::eye::PerEye;
use xr::error::XrError;
use xr::projection::EyeView;

use super::LayerError;
use super::matrix::{Matrix, view_projection};

/// What a frame's draw of the layer gives the session.
pub(crate) enum Outcome<V> {
    /// The layer, with each eye's view.
    Layer(V),
    /// No layer in this frame.
    NoLayer,
    /// The layer failed, so the session shows no controller model from then on.
    Off(LayerError),
    /// The runtime lost the session.
    Lost(XrError),
}

/// What the result `drawn` of a frame's draw gives the session: a lost session ends it, and any
/// other failure turns the layer off.
pub(crate) fn outcome<V>(drawn: Result<Option<V>, LayerError>) -> Outcome<V> {
    match drawn {
        Ok(Some(views)) => Outcome::Layer(views),
        Ok(None) => Outcome::NoLayer,
        Err(LayerError::Xr(error)) if error.is_loss() => Outcome::Lost(error),
        Err(error) => Outcome::Off(error),
    }
}

/// Each eye's view of a frame and its matrix into clip space.
pub(crate) struct Located {
    pub(crate) views: PerEye<EyeView>,
    pub(crate) view_projections: PerEye<Matrix>,
}

/// Why a location of the eye views gives the layer no views.
pub(crate) enum NoViews {
    /// The runtime marks a view's position or orientation invalid.
    NotLocated,
    /// The location failed and the session runs on.
    Failed(XrError),
    /// An eye view's field of view has no width or no height.
    NoProjection,
    /// The runtime lost the session.
    Lost(XrError),
}

/// Each eye's view of `located` with its matrix into clip space.
///
/// # Errors
///
/// Returns the [`NoViews`] that names why `located` gives the layer no views.
pub(crate) fn views(located: Result<Option<PerEye<EyeView>>, XrError>) -> Result<Located, NoViews> {
    let views = match located {
        Ok(Some(views)) => views,
        Ok(None) => return Err(NoViews::NotLocated),
        Err(error) if error.is_loss() => return Err(NoViews::Lost(error)),
        Err(error) => return Err(NoViews::Failed(error)),
    };
    let (Some(left), Some(right)) = (view_projection(views.left), view_projection(views.right))
    else {
        return Err(NoViews::NoProjection);
    };
    Ok(Located { views, view_projections: PerEye { left, right } })
}

#[cfg(test)]
mod tests;
