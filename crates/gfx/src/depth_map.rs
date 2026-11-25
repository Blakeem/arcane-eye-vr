//! The depth map: the uncrossed disparity of what each cell of a frame's left half shows and
//! whether that is the interface, the scene or a covered scene, found on the GPU, handed to the
//! CPU with no wait and written into the depth map section that a viewing program reads.
//!
//! The layer spans an entire frame, and each half of the distance texture spans one eye's entire
//! picture. A cell's place is its middle, as shares of the left half. A frame that draws the layer
//! draws it into its interface area, moved right by half the UI disparity. It shows the interface
//! at a place whose layer texel, the texel at the game place of the place moved left by half the
//! UI disparity, has an alpha above 0.25, with the negative of the UI disparity as its uncrossed
//! disparity.
//! Every other place shows the scene: in a frame the game rendered side by side, the depth offset
//! minus the crossed disparity of the distance texture's value at that place of its left half, or
//! the depth offset for a value that is 0, negative or not finite. In a pair made from a flat
//! picture the scene's disparity is 0. In a frame whose interface area is not the entire half, a
//! scene place whose own layer texel has an alpha above 0.25 is a covered scene, since the game
//! gives a click there to that interface element, which the frame drew elsewhere.
//!
//! A started map draws one texel per cell into a target texture and copies it into a staging
//! texture of its own, which a later frame maps with no wait, in the crate's `readback` module.
//! The draw puts every changed value of the context back before it returns, as the layer draw
//! does. Each finished map goes into the section under the sequence rule of the head pose.

use ipc_layout::viewer_header::WORD_SIZE;
use ipc_layout::viewer_header::depth_map::{
    Grid, InterfaceArea, LAYOUT_VERSION, LAYOUT_VERSION_OFFSET, MAGIC, MAGIC_OFFSET, MapInfo,
    SEQUENCE_OFFSET, SIZE, write_sequences,
};
use windows::Win32::Graphics::Direct3D11::{ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D};
use windows::core::HRESULT;

use crate::raw::depth_map::Pipeline;
use crate::raw::row::{ReadSteps, Size, Target, Texels, sampled_view};
use crate::raw::section::{Section, SectionFault};
use crate::raw::texture_desc;
use crate::readback::Readback;

/// The section's size as `CreateFileMappingW` takes it.
const SECTION_SIZE: u32 = 524_328;
const _: () = assert!(SECTION_SIZE as usize == SIZE);

/// The depth map step that failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MapStep {
    VertexShader,
    PixelShader,
    Constants,
    Target,
    TargetView,
    LayerView,
    DistanceView,
    Staging,
    Map,
    Rows,
}

impl MapStep {
    const fn as_str(self) -> &'static str {
        match self {
            Self::VertexShader => "creating the depth map's vertex shader",
            Self::PixelShader => "creating the depth map's pixel shader",
            Self::Constants => "creating the depth map's constant buffer",
            Self::Target => "creating the depth map's target texture",
            Self::TargetView => "creating a view of the depth map's target texture",
            Self::LayerView => "creating a view of the layer texture for the depth map",
            Self::DistanceView => "creating a view of the distance texture for the depth map",
            Self::Staging => "creating a staging texture of the depth map",
            Self::Map => "mapping a staging texture of the depth map",
            Self::Rows => "reading the rows of a mapped staging texture of the depth map",
        }
    }
}

/// A failed depth map step and the code its call returned.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{} failed with {code}", step.as_str())]
pub struct MapError {
    pub step: MapStep,
    pub code: HRESULT,
}

/// Why the depth map section could not be made.
#[derive(Debug, thiserror::Error)]
pub enum SectionError {
    #[error("could not create the depth map's file mapping: {0}")]
    Create(#[source] windows::core::Error),
    #[error("could not map a view of the depth map: {0}")]
    Map(#[source] windows::core::Error),
}

impl From<SectionFault> for SectionError {
    fn from(fault: SectionFault) -> Self {
        match fault {
            SectionFault::Create(source) => Self::Create(source),
            SectionFault::Map(source) => Self::Map(source),
        }
    }
}

/// The layer a frame draws into each half, its UI disparity and its interface area.
#[derive(Clone, Copy, Debug)]
pub struct MapUi<'frame> {
    pub layer: &'frame ID3D11Texture2D,
    /// The crossed disparity of the interface as a share, 0 or more. The UI draw moves the layer
    /// half of it to the right in the left half.
    pub disparity: f32,
    /// The rectangle of each half the UI draw put the layer into before that move.
    pub area: InterfaceArea,
}

/// Where a frame's scene disparity comes from.
#[derive(Clone, Copy, Debug)]
pub enum MapScene<'frame> {
    /// The pair the core makes from a flat picture, whose halves it moves neither: the disparity 0
    /// and no depth offset.
    Flat,
    /// A frame the game rendered side by side.
    Rendered {
        distances: &'frame ID3D11Texture2D,
        /// The crossed disparity of a scene at the distance texture's value 1, so a value `v` has
        /// the crossed disparity `crossed / v`.
        crossed: f32,
        /// The depth offset in use as a share.
        offset: f32,
    },
}

impl MapScene<'_> {
    /// The depth offset the map's header holds.
    pub const fn offset(&self) -> f32 {
        match *self {
            Self::Flat => 0.0,
            Self::Rendered { offset, .. } => offset,
        }
    }
}

/// What a start of the depth map measures and the present number of its frame.
#[derive(Clone, Copy, Debug)]
pub struct MapStart<'frame> {
    pub grid: Grid,
    pub present: u32,
    /// The layer the frame draws, or `None` for a frame that draws none.
    pub ui: Option<MapUi<'frame>>,
    pub scene: MapScene<'frame>,
}

/// The grid over `layer`, the UI texture, by [`Grid::covering`] of its size.
pub fn layer_grid(layer: &ID3D11Texture2D) -> Grid {
    let desc = texture_desc(layer);
    Grid::covering(desc.Width, desc.Height)
}

/// The depth map's objects on one device.
pub struct DepthMapRead {
    pipeline: Pipeline,
    /// The target texture of the last start, made again when the grid changes.
    target: Option<Target>,
    /// The staging textures and the waiting maps, each with its header's values.
    readback: Readback<MapInfo>,
}

impl DepthMapRead {
    /// Creates the map's shaders and constant buffer on `device`.
    ///
    /// # Errors
    ///
    /// Returns the create call that failed.
    pub fn create(device: &ID3D11Device) -> Result<Self, MapError> {
        Ok(Self {
            pipeline: Pipeline::create(device)?,
            target: None,
            readback: Readback::new(device),
        })
    }

    /// Whether the map's objects belong to `device`.
    pub fn belongs_to(&self, device: &ID3D11Device) -> bool {
        self.pipeline.device() == device
    }

    /// Starts the map of `start` through `context`, the immediate context of the map's device.
    /// Starts nothing while every slot waits for the GPU. The distance texture passed
    /// [`check_distance`](crate::nearest::check_distance), and both textures can be sampled.
    ///
    /// # Errors
    ///
    /// Returns the view or the texture that could not be created, which is the target for a grid
    /// of no cell. The start then started no map.
    pub fn start(
        &mut self,
        context: &ID3D11DeviceContext,
        start: &MapStart<'_>,
    ) -> Result<(), MapError> {
        if !self.readback.has_free() {
            return Ok(());
        }
        let device = self.pipeline.device();
        let grid = start.grid;
        let size = Size::new(MapStep::Target, grid.columns().into(), grid.rows().into())?;
        let layer = match start.ui {
            Some(ui) => Some(sampled_view(device, MapStep::LayerView, ui.layer)?),
            None => None,
        };
        let distances = match start.scene {
            MapScene::Rendered { distances, .. } => {
                Some(sampled_view(device, MapStep::DistanceView, distances)?)
            }
            MapScene::Flat => None,
        };
        let target = match self.target.take().filter(|target| target.size() == size) {
            Some(target) => target,
            None => Target::create(device, size, MapStep::Target, MapStep::TargetView)?,
        };
        let target = self.target.insert(target);
        self.pipeline.draw(context, target, [layer, distances], start);
        let area = start.ui.map_or(InterfaceArea::FULL, |ui| ui.area);
        let info =
            MapInfo { grid, depth_offset: start.scene.offset(), present: start.present, area };
        self.readback.queue(context, target, MapStep::Staging, info)
    }

    /// Writes each map the GPU finished into `section`, oldest first, and returns whether it wrote
    /// one. Frees the slot of each finished map and stops at the first map that still waits.
    ///
    /// # Errors
    ///
    /// Returns [`MapStep::Map`] with the code of a map that failed for another reason than a GPU
    /// that still draws, such as a removed device, and [`MapStep::Rows`] for a mapped staging
    /// texture whose row pitch cannot hold its rows.
    pub fn publish_finished(
        &mut self,
        context: &ID3D11DeviceContext,
        section: &MapSection,
    ) -> Result<bool, MapError> {
        let steps = ReadSteps { map: MapStep::Map, rows: MapStep::Rows };
        let written =
            self.readback.newest(context, steps, |texels, info| section.write(info, texels))?;
        Ok(written.is_some())
    }

    /// Forgets every map that waits for the GPU, so [`Self::publish_finished`] writes no map
    /// started before this call.
    pub fn discard(&mut self) {
        self.readback.discard();
    }
}

/// The core's view of the depth map section, which a viewing program reads.
pub struct MapSection {
    section: Section,
}

impl MapSection {
    /// Creates the section `name`, or opens it when a viewing program or an earlier core still
    /// holds it, and writes its magic value and layout version. Its sequence number continues
    /// from the one the section holds, so the first write of a new section ends with 2.
    ///
    /// # Errors
    ///
    /// Returns [`SectionError`] when the section cannot be created or mapped.
    pub fn create(name: &str) -> Result<Self, SectionError> {
        let section = Section::create(name, SECTION_SIZE)?;
        section.write_words([(MAGIC_OFFSET, MAGIC), (LAYOUT_VERSION_OFFSET, LAYOUT_VERSION)]);
        Ok(Self { section })
    }

    /// Writes the map of no cell, no depth offset and the entire half as its area of the frame of
    /// `present`.
    pub fn write_empty(&self, present: u32) {
        self.write_cells(MapInfo::empty(present), None);
    }

    fn write(&self, info: MapInfo, texels: &Texels<'_>) {
        self.write_cells(info, Some(texels));
    }

    /// Writes the header of `info` and the cells of its grid from `texels`: an odd sequence number
    /// first and the next even number last.
    fn write_cells(&self, info: MapInfo, texels: Option<&Texels<'_>>) {
        let current = self.section.read_word(SEQUENCE_OFFSET).unwrap_or(0);
        let (start, end) = write_sequences(current);
        self.section.write_words((0..).step_by(WORD_SIZE).zip(info.words(start)));
        if let Some(texels) = texels {
            self.write_rows(info.grid, texels);
        }
        self.section.write_words([(SEQUENCE_OFFSET, end)]);
    }

    /// Writes the cells of `grid`, row by row, from the rows of `texels`.
    fn write_rows(&self, grid: Grid, texels: &Texels<'_>) {
        for row in 0..grid.rows() {
            let (Some(first), Some(cells)) = (grid.cell_offset(0, row), texels.row(row)) else {
                return;
            };
            let in_use = cells.iter().take(usize::from(grid.columns()));
            let words = in_use.flat_map(|cell| cell.map(f32::to_bits));
            self.section.write_words((first..).step_by(WORD_SIZE).zip(words));
        }
    }
}
