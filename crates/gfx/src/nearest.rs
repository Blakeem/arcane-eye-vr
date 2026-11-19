//! The nearest read: the smallest value of a distance texture under the covering texels of a layer
//! texture, or that the layer hides the scene, found on the GPU and handed to the CPU with no wait.
//!
//! The layer spans an entire frame, and each half of the distance texture spans one eye's entire
//! picture. The read visits the layer's texels on a grid whose points lie [`SPACING`] texels apart.
//! A texel whose alpha is above 0.25 covers, and the read takes the distance texture's value at the
//! texel's place in each half. Places are shares of each texture's size, so the two textures can
//! differ in size. When every visited texel has an alpha of at least 0.99, the layer hides the
//! scene, whatever the distance texture holds.
//!
//! A started read draws two values per grid column into a row texture, the smallest distance and
//! the smallest alpha, and copies that row into a staging texture of its own, which a later frame
//! maps with no wait, in the crate's `readback` module. The read draws on the context it gets and
//! puts every changed value back before it returns, as the layer draw does.

use windows::Win32::Graphics::Direct3D11::{
    D3D11_BIND_SHADER_RESOURCE, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT, DXGI_FORMAT_R32_FLOAT};
use windows::core::HRESULT;

use crate::raw::nearest::Pipeline;
use crate::raw::row::{Row, Texel, sampled_view};
use crate::raw::texture_desc;
use crate::readback::Readback;

/// The texels between two grid points in each direction, as `SPACING` in `shaders/nearest.slang`.
pub const SPACING: u32 = 8;

/// The smallest alpha of a layer texel that hides the scene under it.
const HIDING: f32 = 0.99;

/// The nearest read step that failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NearestStep {
    VertexShader,
    PixelShader,
    LayerView,
    DistanceView,
    Columns,
    ColumnsView,
    Staging,
    Map,
}

impl NearestStep {
    const fn as_str(self) -> &'static str {
        match self {
            Self::VertexShader => "creating the nearest read's vertex shader",
            Self::PixelShader => "creating the nearest read's pixel shader",
            Self::LayerView => "creating a view of the layer texture",
            Self::DistanceView => "creating a view of the distance texture",
            Self::Columns => "creating the row texture of the grid columns",
            Self::ColumnsView => "creating a view of the row texture of the grid columns",
            Self::Staging => "creating a staging texture of the grid columns",
            Self::Map => "mapping a staging texture of the grid columns",
        }
    }
}

/// A failed nearest read step and the code its call returned.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{} failed with {code}", step.as_str())]
pub struct NearestError {
    pub step: NearestStep,
    pub code: HRESULT,
}

/// Why a texture cannot serve as the distance texture.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum DistanceFault {
    #[error("the distance texture has the format {}, not R32_FLOAT", .0.0)]
    Format(DXGI_FORMAT),
    #[error("the distance texture holds {0} samples per texel, not 1")]
    Samples(u32),
    #[error("the distance texture is {0} texels wide, too narrow for two halves")]
    Width(u32),
    #[error("the distance texture cannot be sampled")]
    NotSampled,
}

/// What a finished read found.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Nearest {
    /// No texel of the layer covers.
    Uncovered,
    /// The smallest distance value under a covering texel, times the scale its start named.
    At(f32),
    /// Every visited texel of the layer has an alpha of at least 0.99, so no scene shows.
    Hidden,
}

/// The checks of `texture` as the distance texture: one sample per texel of `R32_FLOAT`, at least
/// two texels wide and sampled by a shader.
///
/// # Errors
///
/// Returns the first check that fails.
pub fn check_distance(texture: &ID3D11Texture2D) -> Result<(), DistanceFault> {
    let desc = texture_desc(texture);
    let sampled = desc.BindFlags & D3D11_BIND_SHADER_RESOURCE.0.cast_unsigned() != 0;
    if desc.Format != DXGI_FORMAT_R32_FLOAT {
        return Err(DistanceFault::Format(desc.Format));
    }
    if desc.SampleDesc.Count != 1 {
        return Err(DistanceFault::Samples(desc.SampleDesc.Count));
    }
    if desc.Width < 2 {
        return Err(DistanceFault::Width(desc.Width));
    }
    if !sampled {
        return Err(DistanceFault::NotSampled);
    }
    Ok(())
}

/// The nearest read's objects on one device.
pub struct NearestRead {
    pipeline: Pipeline,
    /// The row texture of the last start, made again when the grid's width changes.
    columns: Option<Row>,
    /// The staging textures and the waiting reads, each with the scale its start named.
    readback: Readback<f32>,
}

impl NearestRead {
    /// Creates the read's shaders on `device`.
    ///
    /// # Errors
    ///
    /// Returns the create call that failed.
    pub fn create(device: &ID3D11Device) -> Result<Self, NearestError> {
        Ok(Self {
            pipeline: Pipeline::create(device)?,
            columns: None,
            readback: Readback::new(device),
        })
    }

    /// Whether the read's objects belong to `device`.
    pub fn belongs_to(&self, device: &ID3D11Device) -> bool {
        self.pipeline.device() == device
    }

    /// Starts a read of `distances` under `layer` through `context`, the immediate context of the
    /// read's device, whose result is the smallest value times `scale` or a layer that hides the
    /// scene. Starts nothing while every slot waits for the GPU. `distances` passed
    /// [`check_distance`], and both textures can be sampled.
    ///
    /// # Errors
    ///
    /// Returns the view or the texture that could not be created. The start then started no read.
    pub fn start(
        &mut self,
        context: &ID3D11DeviceContext,
        layer: &ID3D11Texture2D,
        distances: &ID3D11Texture2D,
        scale: f32,
    ) -> Result<(), NearestError> {
        if !self.readback.has_free() {
            return Ok(());
        }
        let device = self.pipeline.device();
        let width = texture_desc(layer).Width.div_ceil(SPACING);
        let layer_view = sampled_view(device, NearestStep::LayerView, layer)?;
        let distance_view = sampled_view(device, NearestStep::DistanceView, distances)?;
        let columns = match self.columns.take().filter(|columns| columns.width() == width) {
            Some(columns) => columns,
            None => Row::create(device, width, NearestStep::Columns, NearestStep::ColumnsView)?,
        };
        let columns = self.columns.insert(columns);
        self.pipeline.draw(context, columns, &layer_view, &distance_view);
        self.readback.queue(context, columns, NearestStep::Staging, scale)
    }

    /// The result of the newest read that the GPU finished, or `None` when no read finished since
    /// the last call. Frees the slot of each finished read, oldest first, and stops at the first
    /// read that still waits.
    ///
    /// # Errors
    ///
    /// Returns [`NearestStep::Map`] with the code of a map that failed for another reason than a
    /// GPU that still draws, such as a removed device.
    pub fn newest(
        &mut self,
        context: &ID3D11DeviceContext,
    ) -> Result<Option<Nearest>, NearestError> {
        self.readback.newest(context, NearestStep::Map, found)
    }

    /// Forgets every read that waits for the GPU, so [`Self::newest`] gives no result of a read
    /// started before this call.
    pub fn discard(&mut self) {
        self.readback.discard();
    }
}

/// What the row `columns` of a read whose start named `scale` found. Each column's red channel is
/// the smallest distance under its covering texels, or a negative value when none covers, and its
/// green channel the smallest alpha of its visited texels. A row whose every column has a smallest
/// alpha of at least [`HIDING`] hides the scene. Otherwise the smallest distance that is 0 or more
/// counts, since a column with no covering texel holds a negative distance and a NaN compares
/// false.
fn found(columns: &[Texel], scale: f32) -> Nearest {
    let hides = !columns.is_empty() && columns.iter().all(|&[_, alpha]| alpha >= HIDING);
    if hides {
        return Nearest::Hidden;
    }
    let distances = columns.iter().map(|&[distance, _]| distance);
    let smallest = distances.filter(|distance| *distance >= 0.0).reduce(f32::min);
    smallest.map_or(Nearest::Uncovered, |distance| Nearest::At(distance * scale))
}

#[cfg(test)]
mod tests {
    use super::{Nearest, Texel, found};

    const OPAQUE: f32 = 1.0;
    const HALF: f32 = 0.5;
    const NONE_COVERS: f32 = -1.0;

    fn row(columns: &[(f32, f32)]) -> Vec<Texel> {
        columns.iter().map(|&column| Texel::from(column)).collect()
    }

    #[test]
    fn the_smallest_distance_skips_the_columns_with_no_covering_texel_and_nans() {
        let columns = row(&[(0.5, HALF), (NONE_COVERS, 0.0), (0.25, HALF), (f32::NAN, HALF)]);

        assert_eq!(found(&columns, 10.0), Nearest::At(2.5));
        assert_eq!(found(&row(&[(0.0, HALF), (0.5, OPAQUE)]), 10.0), Nearest::At(0.0));
    }

    #[test]
    fn a_row_with_no_covering_texel_finds_no_scene_under_the_layer() {
        let none_covers = row(&[(NONE_COVERS, 0.0), (NONE_COVERS, 0.2)]);

        assert_eq!(found(&none_covers, 10.0), Nearest::Uncovered);
        assert_eq!(found(&[], 10.0), Nearest::Uncovered);
    }

    #[test]
    fn a_row_whose_every_column_has_an_alpha_of_0_99_or_more_hides_the_scene() {
        assert_eq!(found(&row(&[(0.1, OPAQUE), (0.5, 0.99)]), 10.0), Nearest::Hidden);
        assert_eq!(found(&row(&[(0.1, OPAQUE), (0.5, 0.989)]), 10.0), Nearest::At(1.0));
        assert_eq!(found(&row(&[(0.1, OPAQUE), (0.5, f32::NAN)]), 10.0), Nearest::At(1.0));
    }
}
