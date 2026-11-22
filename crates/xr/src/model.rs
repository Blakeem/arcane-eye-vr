//! The render models of the devices the user holds, as the runtime lists them through
//! `XR_EXT_interaction_render_model`: each model's GLB bytes and animatable node names once, and in
//! each frame its pose and the state of each animatable node.
//!
//! The session keeps one model for each id the runtime lists. The specification forbids choosing
//! anything by a model's id, so the id serves only as the model's key. A model whose call fails is
//! dropped and is not created again while the runtime lists its id.

use std::cell::Cell;
use std::mem;

use openxr::{D3D11, Posef, Space, Time, sys};
use tracing::{info, warn};

use crate::error::{XrError, XrStep};
use crate::raw::render_model::{NodeName, RenderModel, Tables, interaction_model_ids};

mod decide;

use decide::Listing;

/// A render model's key, unique in the session.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ModelKey(u64);

/// Whether an animatable node and its descendants are drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeVisibility {
    Shown,
    Hidden,
}

/// The state of one animatable node in one frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NodeState {
    /// The node's pose in its parent node's space, which replaces the node's own transform.
    pub pose: Posef,
    pub visibility: NodeVisibility,
}

/// A model the runtime listed since the last [`crate::session::Session::take_model_changes`].
#[derive(Debug, PartialEq, Eq)]
pub struct ArrivedModel {
    pub key: ModelKey,
    /// The model's glTF 2.0 binary (GLB).
    pub glb: Vec<u8>,
    /// The names of the model's animatable nodes, in the order of each frame's node states.
    pub node_names: Vec<String>,
}

/// The models that arrived and the keys of the models that left since the last call. A model that
/// arrived and left between two calls is in neither list.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ModelChanges {
    pub arrived: Vec<ArrivedModel>,
    pub left: Vec<ModelKey>,
}

impl ModelChanges {
    /// Records that the model of `key` left.
    fn leave(&mut self, key: ModelKey) {
        let count = self.arrived.len();
        self.arrived.retain(|model| model.key != key);
        if self.arrived.len() == count {
            self.left.push(key);
        }
    }
}

/// One kept model in one frame.
#[derive(Debug, PartialEq)]
pub struct ModelFrame {
    pub key: ModelKey,
    /// `None` while the model's space is not tracked, so the model is not drawn.
    pub tracked: Option<TrackedModel>,
}

/// A tracked model's pose in the LOCAL space and the state of each animatable node, in the order
/// of its node names.
#[derive(Debug, PartialEq)]
pub struct TrackedModel {
    pub pose: Posef,
    pub nodes: Vec<NodeState>,
}

/// Why a model was dropped.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub(crate) enum ModelError {
    #[error(transparent)]
    Call(#[from] XrError),
    #[error("the name of animatable node {index} is not NUL-terminated UTF-8")]
    NodeName { index: usize },
    #[error("the runtime gave {states} node states for {names} node names")]
    NodeCount { names: usize, states: usize },
}

/// One kept model: its handle, its space and its node counts.
struct Kept {
    id: sys::RenderModelIdEXT,
    key: ModelKey,
    space: Space,
    model: RenderModel,
    /// The animatable node count the model's properties give, the count of its node states.
    node_count: u32,
    name_count: usize,
}

impl Kept {
    fn frame(&self, base: &Space, time: Time) -> Result<ModelFrame, ModelError> {
        let location = XrError::check(XrStep::LocateSpace, self.space.locate(base, time))?;
        let Some(pose) = decide::tracked_pose(location) else {
            return Ok(ModelFrame { key: self.key, tracked: None });
        };
        let states = self.model.node_states(time, self.node_count);
        let states = XrError::check(XrStep::ReadRenderModelState, states)?;
        let nodes = decide::node_states(self.name_count, &states)?;
        Ok(ModelFrame { key: self.key, tracked: Some(TrackedModel { pose, nodes }) })
    }
}

/// The session's render models and the changes its caller has not taken.
pub(crate) struct Models {
    tables: Tables,
    /// The ids of the last list, including those of dropped models.
    listed: Vec<sys::RenderModelIdEXT>,
    kept: Vec<Kept>,
    changes: ModelChanges,
    listing: Cell<Listing>,
}

impl Models {
    pub(crate) fn new(tables: Tables) -> Self {
        Self {
            tables,
            listed: Vec::new(),
            kept: Vec::new(),
            changes: ModelChanges::default(),
            listing: Cell::new(Listing::BeforeSync),
        }
    }

    /// Records an action sync, so the next [`Models::list_when_due`] lists the models once.
    pub(crate) fn synced(&self) {
        self.listing.set(self.listing.get().synced());
    }

    /// Lists the models once after the first action sync, unless a change event listed them, and
    /// again after a failed list.
    pub(crate) fn list_when_due(&mut self, session: &openxr::Session<D3D11>) {
        if self.listing.get().is_due() {
            self.refresh(session);
        }
    }

    /// Lists the ids, creates the model of each new id and destroys the model of each missing id.
    /// A failed list keeps the models as they are until a later list succeeds.
    pub(crate) fn refresh(&mut self, session: &openxr::Session<D3D11>) {
        let listed = interaction_model_ids(session, self.tables);
        let ids = match XrError::check(XrStep::ListRenderModels, listed) {
            Ok(ids) => ids,
            Err(error) => {
                if self.listing.get().warns_on_failure() {
                    warn!(
                        error = %error,
                        "the render models stay as they were until a list succeeds"
                    );
                }
                self.listing.set(Listing::Retry);
                return;
            }
        };
        if self.listing.get() == Listing::Retry {
            info!("the render models are listed again");
        }
        self.listing.set(Listing::Listed);
        let decide::Diff { create, destroy } = decide::diff(&self.listed, &ids);
        for id in destroy {
            self.remove(id);
        }
        for id in create {
            self.add(session, id);
        }
        self.listed = ids;
    }

    fn remove(&mut self, id: sys::RenderModelIdEXT) {
        let Some(index) = self.kept.iter().position(|kept| kept.id == id) else {
            return;
        };
        let kept = self.kept.remove(index);
        info!(key = ?kept.key, "a render model left");
        self.changes.leave(kept.key);
    }

    fn add(&mut self, session: &openxr::Session<D3D11>, id: sys::RenderModelIdEXT) {
        let key = ModelKey(id.into_raw());
        match arrive(session, self.tables, id, key) {
            Ok((kept, arrived)) => {
                info!(
                    key = ?key,
                    glb_bytes = arrived.glb.len(),
                    node_count = arrived.node_names.len(),
                    node_names = ?arrived.node_names,
                    "a render model arrived"
                );
                self.kept.push(kept);
                self.changes.arrived.push(arrived);
            }
            Err(error) => warn_dropped(key, error),
        }
    }

    /// The models that arrived and left since the last call.
    pub(crate) fn take_changes(&mut self) -> ModelChanges {
        mem::take(&mut self.changes)
    }

    /// Each kept model at `time`, with its pose in `base`. A model whose read fails logs a
    /// warning and leaves.
    pub(crate) fn locate(&mut self, base: &Space, time: Time) -> Vec<ModelFrame> {
        let (kept, changes) = (&mut self.kept, &mut self.changes);
        let mut frames = Vec::with_capacity(kept.len());
        kept.retain(|model| match model.frame(base, time) {
            Ok(frame) => {
                frames.push(frame);
                true
            }
            Err(error) => {
                warn_dropped(model.key, error);
                changes.leave(model.key);
                false
            }
        });
        frames
    }
}

/// The one line of a model that a failed call dropped.
fn warn_dropped(key: ModelKey, error: ModelError) {
    warn!(key = ?key, error = %error, "the render model failed, so it is dropped");
}

/// Creates the model of `id` and reads its GLB bytes and node names, in the specification's order.
fn arrive(
    session: &openxr::Session<D3D11>,
    tables: Tables,
    id: sys::RenderModelIdEXT,
    key: ModelKey,
) -> Result<(Kept, ArrivedModel), ModelError> {
    let model =
        XrError::check(XrStep::CreateRenderModel, RenderModel::create(session, tables, id))?;
    let properties = XrError::check(XrStep::ReadRenderModelProperties, model.properties())?;
    let (cache_id, node_count) = properties;
    let (glb, names) = read_asset(&model, cache_id, node_count)?;
    let node_names = decide::node_names(&names)?;
    let space = XrError::check(XrStep::CreateRenderModelSpace, model.create_space())?;
    let name_count = node_names.len();
    let kept = Kept { id, key, space, model, node_count, name_count };
    Ok((kept, ArrivedModel { key, glb, node_names }))
}

/// The GLB bytes and the raw node names of the asset of `cache_id`. The asset is destroyed on
/// return, since the model's space and node states need no asset.
fn read_asset(
    model: &RenderModel,
    cache_id: sys::UuidEXT,
    node_count: u32,
) -> Result<(Vec<u8>, Vec<NodeName>), XrError> {
    let asset = XrError::check(XrStep::CreateRenderModelAsset, model.create_asset(cache_id))?;
    let glb = XrError::check(XrStep::ReadRenderModelAsset, asset.glb())?;
    let names = XrError::check(XrStep::ReadRenderModelNodeNames, asset.node_names(node_count))?;
    Ok((glb, names))
}

#[cfg(test)]
mod tests;
