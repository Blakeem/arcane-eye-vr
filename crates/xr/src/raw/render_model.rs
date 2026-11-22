//! The calls of `XR_EXT_render_model` and `XR_EXT_interaction_render_model`: the list of the
//! interaction render model ids, a render model and its asset, the model's space and its node
//! states.
//!
//! openxr 0.22 loads the two function tables and has no safe call for them.

use std::ffi::c_char;
use std::ptr;

use openxr::sys::{self, Handle as _};
use openxr::{D3D11, Instance, Space, Time};
use tracing::warn;

/// The bytes of an animatable node's name as the runtime writes it, NUL-terminated.
pub(crate) type NodeName = [c_char; sys::MAX_RENDER_MODEL_ASSET_NODE_NAME_SIZE_EXT];

/// The function tables of the two extensions.
#[derive(Clone, Copy)]
pub(crate) struct Tables {
    model: openxr::raw::RenderModelEXT,
    interaction: openxr::raw::InteractionRenderModelEXT,
}

impl Tables {
    /// The tables of `instance`, or `None` when the instance did not enable both extensions.
    pub(crate) fn of(instance: &Instance) -> Option<Self> {
        let exts = instance.exts();
        Some(Self {
            model: exts.ext_render_model?,
            interaction: exts.ext_interaction_render_model?,
        })
    }
}

/// The runtime's `code`, or the error a failure code is.
fn checked(code: sys::Result) -> openxr::Result<()> {
    if code.into_raw() < 0 {
        return Err(code);
    }
    Ok(())
}

/// The ids of the interaction render models the runtime lists for `session`.
pub(crate) fn interaction_model_ids(
    session: &openxr::Session<D3D11>,
    tables: Tables,
) -> openxr::Result<Vec<sys::RenderModelIdEXT>> {
    let info = sys::InteractionRenderModelIdsEnumerateInfoEXT {
        ty: sys::InteractionRenderModelIdsEnumerateInfoEXT::TYPE,
        next: ptr::null(),
    };
    let enumerate = tables.interaction.enumerate_interaction_render_model_ids;
    let mut count = 0_u32;
    // SAFETY: `session` is live, `info` and `count` live for the call, and a capacity of 0 with a
    // null array asks only for the count.
    let sized =
        unsafe { enumerate(session.as_raw(), &raw const info, 0, &raw mut count, ptr::null_mut()) };
    checked(sized)?;
    if count == 0 {
        return Ok(Vec::new());
    }
    let mut ids = vec![sys::RenderModelIdEXT::default(); count as usize];
    // SAFETY: `ids` holds `count` ids, the capacity the call is given, and it lives for the call.
    let filled = unsafe {
        enumerate(session.as_raw(), &raw const info, count, &raw mut count, ids.as_mut_ptr())
    };
    checked(filled)?;
    ids.truncate(count as usize);
    Ok(ids)
}

/// A render model handle, destroyed on drop. It holds the session, so the session's handle
/// outlives it.
pub(crate) struct RenderModel {
    handle: sys::RenderModelEXT,
    tables: Tables,
    session: openxr::Session<D3D11>,
}

impl RenderModel {
    /// Creates the render model of `id`, an id the runtime listed for `session`, asking for a
    /// GLB that needs no glTF extension.
    pub(crate) fn create(
        session: &openxr::Session<D3D11>,
        tables: Tables,
        id: sys::RenderModelIdEXT,
    ) -> openxr::Result<Self> {
        let info = sys::RenderModelCreateInfoEXT {
            ty: sys::RenderModelCreateInfoEXT::TYPE,
            next: ptr::null(),
            render_model_id: id,
            gltf_extension_count: 0,
            gltf_extensions: ptr::null(),
        };
        let mut handle = sys::RenderModelEXT::NULL;
        let create = tables.model.create_render_model;
        // SAFETY: `session` is live, and `info` and `handle` live for the call.
        let code = unsafe { create(session.as_raw(), &raw const info, &raw mut handle) };
        checked(code)?;
        Ok(Self { handle, tables, session: session.clone() })
    }

    /// The model's asset cache id and its animatable node count.
    pub(crate) fn properties(&self) -> openxr::Result<(sys::UuidEXT, u32)> {
        let info = sys::RenderModelPropertiesGetInfoEXT {
            ty: sys::RenderModelPropertiesGetInfoEXT::TYPE,
            next: ptr::null(),
        };
        let mut properties = sys::RenderModelPropertiesEXT {
            ty: sys::RenderModelPropertiesEXT::TYPE,
            next: ptr::null_mut(),
            cache_id: sys::UuidEXT { data: [0; sys::UUID_SIZE] },
            animatable_node_count: 0,
        };
        let get = self.tables.model.get_render_model_properties;
        // SAFETY: `self.handle` is live, and `info` and `properties` live for the call.
        let code = unsafe { get(self.handle, &raw const info, &raw mut properties) };
        checked(code)?;
        Ok((properties.cache_id, properties.animatable_node_count))
    }

    /// Creates the space at the model's origin.
    pub(crate) fn create_space(&self) -> openxr::Result<Space> {
        let info = sys::RenderModelSpaceCreateInfoEXT {
            ty: sys::RenderModelSpaceCreateInfoEXT::TYPE,
            next: ptr::null(),
            render_model: self.handle,
        };
        let mut handle = sys::Space::NULL;
        let create = self.tables.model.create_render_model_space;
        // SAFETY: the session and `self.handle` are live, and `info` and `handle` live for the
        // call.
        let code = unsafe { create(self.session.as_raw(), &raw const info, &raw mut handle) };
        checked(code)?;
        // SAFETY: the runtime gave `handle` as a space of the session, and `Space` only locates and
        // destroys it.
        Ok(unsafe { Space::reference_from_raw(self.session.clone(), handle) })
    }

    /// Creates the handle of the asset with `cache_id`, the id [`RenderModel::properties`] gave.
    pub(crate) fn create_asset(&self, cache_id: sys::UuidEXT) -> openxr::Result<Asset<'_>> {
        let info = sys::RenderModelAssetCreateInfoEXT {
            ty: sys::RenderModelAssetCreateInfoEXT::TYPE,
            next: ptr::null(),
            cache_id,
        };
        let mut handle = sys::RenderModelAssetEXT::NULL;
        let create = self.tables.model.create_render_model_asset;
        // SAFETY: the session is live, and `info` and `handle` live for the call.
        let code = unsafe { create(self.session.as_raw(), &raw const info, &raw mut handle) };
        checked(code)?;
        Ok(Asset { handle, model: self })
    }

    /// The states of the model's `count` animatable nodes at `time`, in the order of the node
    /// names.
    pub(crate) fn node_states(
        &self,
        time: Time,
        count: u32,
    ) -> openxr::Result<Vec<sys::RenderModelNodeStateEXT>> {
        if count == 0 {
            return Ok(Vec::new());
        }
        let info = sys::RenderModelStateGetInfoEXT {
            ty: sys::RenderModelStateGetInfoEXT::TYPE,
            next: ptr::null(),
            display_time: time,
        };
        let mut states = vec![sys::RenderModelNodeStateEXT::default(); count as usize];
        let mut state = sys::RenderModelStateEXT {
            ty: sys::RenderModelStateEXT::TYPE,
            next: ptr::null_mut(),
            node_state_count: count,
            node_states: states.as_mut_ptr(),
        };
        let get = self.tables.model.get_render_model_state;
        // SAFETY: `self.handle` is live, `state` points to `count` node states, and `info`, `state`
        // and `states` live for the call.
        let code = unsafe { get(self.handle, &raw const info, &raw mut state) };
        checked(code)?;
        Ok(states)
    }
}

impl Drop for RenderModel {
    fn drop(&mut self) {
        let destroy = self.tables.model.destroy_render_model;
        // SAFETY: `self.handle` is live and is not used after the call.
        let code = unsafe { destroy(self.handle) };
        if let Err(code) = checked(code) {
            warn!(code = ?code, "the runtime did not destroy a render model");
        }
    }
}

/// A render model asset handle, destroyed on drop.
pub(crate) struct Asset<'a> {
    handle: sys::RenderModelAssetEXT,
    model: &'a RenderModel,
}

impl Asset<'_> {
    /// The asset's GLB bytes.
    pub(crate) fn glb(&self) -> openxr::Result<Vec<u8>> {
        let info = sys::RenderModelAssetDataGetInfoEXT {
            ty: sys::RenderModelAssetDataGetInfoEXT::TYPE,
            next: ptr::null(),
        };
        let mut data = sys::RenderModelAssetDataEXT {
            ty: sys::RenderModelAssetDataEXT::TYPE,
            next: ptr::null_mut(),
            buffer_capacity_input: 0,
            buffer_count_output: 0,
            buffer: ptr::null_mut(),
        };
        let get = self.model.tables.model.get_render_model_asset_data;
        // SAFETY: `self.handle` is live, `info` and `data` live for the call, and a capacity of 0
        // with a null buffer asks only for the size.
        let sized = unsafe { get(self.handle, &raw const info, &raw mut data) };
        checked(sized)?;
        let mut bytes = vec![0_u8; data.buffer_count_output as usize];
        data.buffer_capacity_input = data.buffer_count_output;
        data.buffer = bytes.as_mut_ptr();
        // SAFETY: `data.buffer` points to `bytes`, which holds the capacity the call is given and
        // lives for the call.
        let filled = unsafe { get(self.handle, &raw const info, &raw mut data) };
        checked(filled)?;
        bytes.truncate(data.buffer_count_output as usize);
        Ok(bytes)
    }

    /// The names of the model's `count` animatable nodes, the count
    /// [`RenderModel::properties`] gave.
    pub(crate) fn node_names(&self, count: u32) -> openxr::Result<Vec<NodeName>> {
        if count == 0 {
            return Ok(Vec::new());
        }
        let info = sys::RenderModelAssetPropertiesGetInfoEXT {
            ty: sys::RenderModelAssetPropertiesGetInfoEXT::TYPE,
            next: ptr::null(),
        };
        let empty = sys::RenderModelAssetNodePropertiesEXT {
            unique_name: [0; sys::MAX_RENDER_MODEL_ASSET_NODE_NAME_SIZE_EXT],
        };
        let mut nodes = vec![empty; count as usize];
        let mut properties = sys::RenderModelAssetPropertiesEXT {
            ty: sys::RenderModelAssetPropertiesEXT::TYPE,
            next: ptr::null_mut(),
            node_property_count: count,
            node_properties: nodes.as_mut_ptr(),
        };
        let get = self.model.tables.model.get_render_model_asset_properties;
        // SAFETY: `self.handle` is live, `properties` points to `count` node properties, and
        // `info`, `properties` and `nodes` live for the call.
        let code = unsafe { get(self.handle, &raw const info, &raw mut properties) };
        checked(code)?;
        Ok(nodes.into_iter().map(|node| node.unique_name).collect())
    }
}

impl Drop for Asset<'_> {
    fn drop(&mut self) {
        let destroy = self.model.tables.model.destroy_render_model_asset;
        // SAFETY: `self.handle` is live and is not used after the call.
        let code = unsafe { destroy(self.handle) };
        if let Err(code) = checked(code) {
            warn!(code = ?code, "the runtime did not destroy a render model asset");
        }
    }
}
