//! The controller models of the headset session: every tracked controller model the runtime gives,
//! drawn into one projection layer, which the compositor blends over the frame's layer and under
//! the panels.
//!
//! `load.rs` reads a model's GLB and makes its meshes and textures once, `scene.rs` places its
//! nodes in each frame, `store.rs` keeps the models by their keys, `matrix.rs` holds the matrices
//! and `decide.rs` the pure decisions of the layer's failures. This module creates the layer's
//! swapchains when it first draws a model, draws each eye's image and logs what the layer did. A
//! frame that the runtime does not show, or with no tracked model or no located eye views, has no
//! layer. A failure of the layer leaves the session with no controller model, and the screen, the
//! menu, the laser and the pointer go on.

mod decide;
mod load;
mod matrix;
mod scene;
mod store;

use std::error::Error;
use std::time::Duration;

use diag::error_chain;
use diag::rate::RateLimit;
use diag::warn_every;
use gfx::mesh::{Item, MeshDraw, MeshError, MeshTarget};
use stereo_core::eye::{Eye, PerEye};
use tracing::{info, warn};
use viewer_client::Device;
use windows::Win32::Graphics::Direct3D11::ID3D11DeviceContext;
use xr::error::XrError;
use xr::format::Sampling;
use xr::model::{ModelChanges, ModelKey};
use xr::projection::EyeView;
use xr::session::{FrameTiming, ModelLayer, Session};
use xr::swapchain::Swapchains;

use self::decide::{Located, NoViews, Outcome};
use self::load::{Arrived, Loaded, Read};
use self::matrix::Matrix;
use self::store::Store;
use super::image;

/// The largest width and height of the layer's images in pixels. The runtime's recommended size
/// follows its own render scale, and the models cover a small share of the view.
const MAX_SIDE: u32 = 2048;

/// The shortest time between two warnings of eye views that give no layer.
const VIEWS_FAILURE_PERIOD: Duration = Duration::from_secs(10);

/// The size of the layer's images for the runtime's `recommended` size, `(width, height)`: each
/// side held to at most [`MAX_SIDE`] on its own.
pub(crate) const fn layer_size(recommended: (u32, u32)) -> (u32, u32) {
    let (width, height) = recommended;
    let width = if width > MAX_SIDE { MAX_SIDE } else { width };
    let height = if height > MAX_SIDE { MAX_SIDE } else { height };
    (width, height)
}

/// Why the layer stopped.
#[derive(Debug, thiserror::Error)]
enum LayerError {
    #[error("an OpenXR call of the controller models' layer failed")]
    Xr(#[from] XrError),
    #[error("the mesh draw failed")]
    Mesh(#[from] MeshError),
    #[error(
        "the runtime gave the controller models' layer the swapchain image {index}, which its image list lacks"
    )]
    NoImage { index: u32 },
}

/// The controller models and their layer.
pub(crate) struct Models {
    /// `None` once the layer failed, so the session shows no controller model.
    drawing: Option<Drawing>,
    views_failure: RateLimit,
}

impl Models {
    pub(crate) fn new() -> Self {
        Self { drawing: Some(Drawing::new()), views_failure: RateLimit::new() }
    }

    /// Takes the session's model changes and, when the runtime shows the frame of `timing`, draws
    /// every tracked model with `device` and gives the layer. Returns `None` for a frame with no
    /// tracked model, no located eye views or no layer.
    ///
    /// # Errors
    ///
    /// Returns the [`XrError`] of a call that lost the session.
    pub(crate) fn frame(
        &mut self,
        session: &mut Session,
        device: &Device,
        timing: FrameTiming,
    ) -> Result<Option<ModelLayer<'_>>, XrError> {
        // Taken also after a failure, since the session holds each arrived GLB until it is taken.
        let changes = session.take_model_changes();
        let Some(drawing) = &mut self.drawing else {
            return Ok(None);
        };
        let drawn = match drawing.update(changes, device) {
            Ok(()) if timing.should_render => {
                drawing.draw(session, device.context(), timing, &mut self.views_failure)
            }
            Ok(()) => Ok(None),
            Err(error) => Err(error),
        };
        let views = match decide::outcome(drawn) {
            Outcome::Layer(views) => views,
            Outcome::NoLayer => return Ok(None),
            Outcome::Lost(error) => return Err(error),
            Outcome::Off(error) => {
                self.fail(&error);
                return Ok(None);
            }
        };
        let layer = self.drawing.as_ref().and_then(|kept| kept.eyes.as_ref());
        Ok(layer.map(|layer| ModelLayer { swapchains: &layer.swapchains, views }))
    }

    /// Logs the failure `error` and leaves the session with no controller model.
    fn fail(&mut self, error: &LayerError) {
        let error = error_chain::chain(error);
        warn!(error = %error, "the controller models' layer failed, so no controller model shows");
        self.drawing = None;
    }
}

/// What one eye's draw of a frame takes: the immediate context, each eye's matrix into clip space
/// and the items.
struct Pass<'a> {
    context: &'a ID3D11DeviceContext,
    view_projections: PerEye<Matrix>,
    items: &'a [Item<'a>],
}

/// The kept models, the mesh draw and the layer's swapchains.
struct Drawing {
    /// `None` until the layer first draws a model.
    eyes: Option<LayerEyes>,
    store: Store<ModelKey, Loaded>,
    /// `None` until the first model reads, so a runtime that gives no readable model makes no draw.
    draw: Option<MeshDraw>,
}

impl Drawing {
    const fn new() -> Self {
        Self { eyes: None, store: Store::new(), draw: None }
    }

    /// Releases each model of `changes` that left and makes each one that arrived with `device`.
    /// A model that cannot be read or made logs a warning and is never drawn. The read and the
    /// upload run on the frame loop's thread, since a model arrives once per controller, and the
    /// runtime reprojects the layers over the frames they take.
    fn update(&mut self, changes: ModelChanges, device: &Device) -> Result<(), LayerError> {
        let ModelChanges { arrived, left } = changes;
        let sources = arrived
            .iter()
            .map(|model| (model.key, model.glb.as_slice(), model.node_names.as_slice()));
        let Arrived { read, unread } = load::read_arrived(sources);
        for (key, error) in &unread {
            warn_unloaded(*key, error);
        }
        let loaded = if read.is_empty() {
            Vec::new()
        } else {
            let draw = match &mut self.draw {
                Some(draw) => draw,
                None => self.draw.insert(MeshDraw::create(device.d3d())?),
            };
            let context = device.context();
            read.into_iter()
                .filter_map(|(key, read)| Some((key, upload_model(draw, context, key, read)?)))
                .collect()
        };
        self.store.apply(&left, loaded);
        Ok(())
    }

    /// Draws every kept model that `session` tracks at the display time of `timing` into each
    /// eye's next image of the layer through `context`, and returns each eye's view. Returns `None`
    /// and draws nothing for a frame with no tracked model or no located eye views, whose warning
    /// `failure` limits.
    fn draw(
        &mut self,
        session: &mut Session,
        context: &ID3D11DeviceContext,
        timing: FrameTiming,
        failure: &mut RateLimit,
    ) -> Result<Option<PerEye<EyeView>>, LayerError> {
        let frames = session.locate_models(timing);
        let Self { eyes, store, draw } = self;
        let mut items: Vec<Item<'_>> = Vec::new();
        for (model, state) in
            store.tracked(frames.iter().map(|frame| (frame.key, frame.tracked.as_ref())))
        {
            model.items(state.pose, &state.nodes, &mut items);
        }
        if items.is_empty() {
            return Ok(None);
        }
        let Some(draw) = draw.as_mut() else {
            return Ok(None);
        };
        let Some(located) = eye_views(session, timing, failure)? else {
            return Ok(None);
        };
        let pass = Pass { context, view_projections: located.view_projections, items: &items };
        render(eyes, draw, session, &pass)?;
        Ok(Some(located.views))
    }
}

/// The read model `read` of `key` on the device of `draw`, or `None` after a warning when it cannot
/// be made.
fn upload_model(
    draw: &MeshDraw,
    context: &ID3D11DeviceContext,
    key: ModelKey,
    read: Read,
) -> Option<Loaded> {
    let model = &read.model;
    let (meshes, materials, textures) =
        (model.meshes.len(), model.materials.len(), model.textures.len());
    let undecoded: Vec<String> = model
        .undecoded
        .iter()
        .map(|image| {
            let mime_type = image.mime_type.as_deref().unwrap_or("no MIME type");
            format!("image {} ({mime_type}): {}", image.image, image.fault)
        })
        .collect();
    info!(key = ?key, meshes, materials, textures, undecoded = ?undecoded, "a controller model is read");
    match Loaded::upload(read, draw, context) {
        Ok(loaded) => Some(loaded),
        Err(error) => {
            warn_unloaded(key, &error);
            None
        }
    }
}

fn warn_unloaded(key: ModelKey, error: &dyn Error) {
    let error = error_chain::chain(error);
    warn!(key = ?key, error = %error, "the controller model cannot be shown, so it is never drawn");
}

/// Each eye's view of the frame of `timing` and its matrix into clip space, or `None` while the
/// views are not located or give no projection, which warns at most once per
/// [`VIEWS_FAILURE_PERIOD`] through `failure`.
///
/// # Errors
///
/// Returns the [`XrError`] of a location that lost the session.
fn eye_views(
    session: &Session,
    timing: FrameTiming,
    failure: &mut RateLimit,
) -> Result<Option<Located>, XrError> {
    match decide::views(session.locate_eye_views(timing)) {
        Ok(located) => Ok(Some(located)),
        Err(NoViews::NotLocated) => Ok(None),
        Err(NoViews::Lost(error)) => Err(error),
        Err(NoViews::Failed(error)) => {
            warn_every!(
                *failure,
                VIEWS_FAILURE_PERIOD,
                error = %error,
                "the eye views could not be located, so no controller model is drawn"
            );
            Ok(None)
        }
        Err(NoViews::NoProjection) => {
            warn_every!(
                *failure,
                VIEWS_FAILURE_PERIOD,
                "an eye view's field of view has no width or no height, so no controller model is drawn"
            );
            Ok(None)
        }
    }
}

/// Draws `pass` into each eye's next image of the layer `eyes` with `draw`, after it creates the
/// layer through `session` when `eyes` is `None`.
fn render(
    eyes: &mut Option<LayerEyes>,
    draw: &mut MeshDraw,
    session: &Session,
    pass: &Pass<'_>,
) -> Result<(), LayerError> {
    let layer = match eyes {
        Some(layer) => layer,
        None => eyes.insert(LayerEyes::create(session, draw)?),
    };
    for eye in [Eye::Left, Eye::Right] {
        layer.draw_eye(draw, eye, pass)?;
    }
    Ok(())
}

/// The layer's swapchains and the mesh draw's view of each of their images. Fields drop in
/// declaration order, so the views go before the swapchains.
struct LayerEyes {
    targets: PerEye<Vec<MeshTarget>>,
    swapchains: Swapchains,
}

impl LayerEyes {
    fn create(session: &Session, draw: &MeshDraw) -> Result<Self, LayerError> {
        let swapchains = session.create_swapchains(layer_size(session.recommended_size()?))?;
        // The mesh draw writes linear color, which a view of the sRGB format stores encoded, as
        // the runtime reads the swapchain's images.
        let format = session.format().target_view(Sampling::SrgbDecoded);
        let targets = |eye: Eye| -> Result<Vec<MeshTarget>, MeshError> {
            swapchains.eye(eye).images().iter().map(|image| draw.target(image, format)).collect()
        };
        let targets = PerEye { left: targets(Eye::Left)?, right: targets(Eye::Right)? };
        Ok(Self { targets, swapchains })
    }

    /// Draws `pass` with `eye`'s matrix into `eye`'s next image and releases it.
    fn draw_eye(
        &mut self,
        draw: &mut MeshDraw,
        eye: Eye,
        pass: &Pass<'_>,
    ) -> Result<(), LayerError> {
        let swapchain = self.swapchains.eye_mut(eye);
        let index = swapchain.acquire()?;
        let view_projection = &pass.view_projections[eye];
        let drawn = image::target(&self.targets[eye], index)
            .ok_or(LayerError::NoImage { index })
            .and_then(|target| Ok(draw.draw(pass.context, target, view_projection, pass.items)?));
        swapchain.release()?;
        drawn
    }
}

#[cfg(test)]
mod tests;
