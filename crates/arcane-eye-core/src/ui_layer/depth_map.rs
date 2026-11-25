//! The depth map of each frame the core publishes while a viewing program's heartbeat runs: how
//! deep each cell of the left half looks after the UI layer's draw.
//!
//! ```text
//! PRESENT CALL SITE, render thread, after the UI layer's frame, before the debug viewer's copy
//!   [sink]      the depth map section and the present number the frame gets, while the viewing
//!               program's heartbeat runs, from the viewer link
//!   [content]   what the UI layer drew into the frame and the interface area it drew the UI
//!               texture into, in `decide`
//!   [inputs]    the UI texture the frame drew, and in a frame the game rendered side by side the
//!               distance texture the UI depth read, or else reads now, and the main view's camera
//!   [finished]  each map the GPU finished, written into the section
//!   [start]     the frame's map on the GPU, or at once a map of no cell
//! ```
//!
//! A frame that holds the game's UI, a frame whose draw did not run to its end, a frame of a flat
//! pair and a frame whose inputs are missing get a map of no cell, which also forgets the maps that
//! wait for the GPU. With no viewing program the core starts no map and forgets the maps that
//! wait. A failed create, draw or map logs a rate-limited warning, so the frame publishes no map
//! and the pair goes on. A failed map also drops the map's objects, and a later frame makes them
//! again. Unlike the UI depth's reads, the map never stops for the session, so a viewing program
//! gets maps again once a failure passes. The UI layer holds the map. In a core with no UI layer
//! the frame handler writes a map of no cell for each frame while a viewing program runs.

mod decide;

use diag::rate::RateLimit;
use diag::warn_every;
use gfx::depth_map::{DepthMapRead, MapError, MapScene, MapSection, MapStart, MapUi, layer_grid};
use ipc_layout::viewer_header::depth_map::{Grid, InterfaceArea};
use windows::Win32::Graphics::Direct3D11::ID3D11Texture2D;

pub(crate) use self::decide::Drawn;
use self::decide::{Content, UiShown, content, crossed_per_unit, ui_disparity};
use super::LOG_PERIOD;
use super::decide::{drawn_offset, halves};
use super::ui_depth::decide::lens;
use super::ui_depth::{DepthInput, DistanceSkip, UiDepth};
use crate::pair::PairState;
use crate::raw::{BackBuffer, Devices};

/// Where a frame's map goes: the depth map section and the present number the frame gets.
pub(crate) struct MapSink<'link> {
    pub(crate) section: &'link MapSection,
    pub(crate) present: u32,
}

/// What the UI layer did in a frame, which its map shows.
pub(crate) struct MapFrame<'frame> {
    pub(crate) pair: PairState,
    /// The draw that ran to its end in the frame, or `None` for none.
    pub(crate) drawn: Option<Drawn>,
    pub(crate) devices: &'frame Devices,
    pub(crate) size: BackBuffer,
    pub(crate) input: &'frame DepthInput,
}

/// The textures a frame's draw read, which its map reads too.
#[derive(Default)]
pub(crate) struct FrameTextures {
    /// The UI texture the frame drew, or `None` in a frame that draws none.
    pub(crate) ui: Option<ID3D11Texture2D>,
    /// The outcome of the UI depth's read of the distance texture in the frame, or `None` when it
    /// read none.
    pub(crate) distances: Option<Result<ID3D11Texture2D, DistanceSkip>>,
}

/// The UI layer's parts that give a frame's map its textures and its UI columns.
pub(crate) struct MapSources<'layer> {
    pub(crate) textures: FrameTextures,
    pub(crate) depth: &'layer mut UiDepth,
}

/// A frame's usable distance texture and the crossed disparity of its unit.
struct Scene {
    distances: ID3D11Texture2D,
    crossed: f32,
}

/// The textures of a frame's map: the UI texture it drew, or `None` for none, and the scene of a
/// frame the game rendered side by side, or `None` for a pair made from a flat picture.
struct Inputs {
    ui: Option<ID3D11Texture2D>,
    scene: Option<Scene>,
}

/// The depth map's render thread state, held in the UI layer.
pub(crate) struct DepthMap {
    /// The map's objects on the device of the last frame that started one.
    read: Option<DepthMapRead>,
    fault_log: RateLimit,
    skip_log: RateLimit,
}

impl DepthMap {
    pub(crate) const fn new() -> Self {
        Self { read: None, fault_log: RateLimit::new(), skip_log: RateLimit::new() }
    }

    /// Writes the map of `frame` into `sink`, from the UI texture and the scene `sources` give.
    pub(crate) fn publish(
        &mut self,
        sink: &MapSink<'_>,
        sources: MapSources<'_>,
        frame: &MapFrame<'_>,
    ) {
        let shown = content(frame.pair, frame.drawn);
        let ui_columns = sources.depth.columns();
        let inputs = self.inputs(shown, sources, frame.input);
        let offset = frame.drawn.map_or(0.0, |drawn| drawn_offset(drawn.draw));
        let area = frame.drawn.map_or(InterfaceArea::FULL, |drawn| drawn.area);
        let ui = UiPlace { columns: ui_columns, area };
        let start =
            inputs.as_ref().map(|inputs| map_start(inputs, ui, frame.size, offset, sink.present));
        self.on_frame(frame.devices, sink, start.as_ref());
    }

    /// One frame through `devices` into `sink`: writes each map the GPU finished and starts the
    /// map of `start`, or writes a map of no cell for `None`.
    pub(crate) fn on_frame(
        &mut self,
        devices: &Devices,
        sink: &MapSink<'_>,
        start: Option<&MapStart<'_>>,
    ) {
        let Some(start) = start else {
            self.forget();
            sink.section.write_empty(sink.present);
            return;
        };
        if let Err(error) = self.measure(devices, sink.section, start) {
            warn_every!(
                self.fault_log,
                LOG_PERIOD,
                error = %error,
                "depth map failed, so the frame publishes no depth map"
            );
        }
    }

    /// Forgets the maps that wait for the GPU, so none of them reaches the section.
    pub(crate) fn forget(&mut self) {
        if let Some(read) = &mut self.read {
            read.discard();
        }
    }

    /// Drops the map's objects, which belong to the device.
    pub(crate) fn release(&mut self) {
        self.read = None;
    }

    fn measure(
        &mut self,
        (device, context): &Devices,
        section: &MapSection,
        start: &MapStart<'_>,
    ) -> Result<(), MapError> {
        let read = match self.read.take().filter(|read| read.belongs_to(device)) {
            Some(read) => read,
            None => DepthMapRead::create(device)?,
        };
        let read = self.read.insert(read);
        if let Err(error) = read.publish_finished(context, section) {
            // The failed map's slot keeps waiting, so only new objects free it.
            self.read = None;
            return Err(error);
        }
        read.start(context, start)
    }

    /// The textures of the map that shows `shown`, or `None` for a map of no cell, as for a frame
    /// whose scene the core cannot read. A frame whose map holds the UI drew the UI texture, so
    /// its draw read it.
    fn inputs(
        &mut self,
        shown: Content,
        MapSources { textures, depth }: MapSources<'_>,
        input: &DepthInput,
    ) -> Option<Inputs> {
        let (ui, rendered) = match shown {
            Content::Empty => return None,
            Content::Flat(ui) => (ui, false),
            Content::Rendered(ui) => (ui, true),
        };
        let ui = match ui {
            UiShown::Drawn => Some(textures.ui?),
            UiShown::NotDrawn => None,
        };
        let scene =
            if rendered { Some(self.scene(depth, textures.distances, input)?) } else { None };
        Some(Inputs { ui, scene })
    }

    /// The scene of a frame the game rendered side by side, at the eye distance of `input` times
    /// the eye scale `depth` says the frame was rendered at, from the UI depth's read `read` of the
    /// distance texture in the frame, or else from a read now. `None` in a frame with no camera,
    /// or with a missing or unusable distance texture.
    fn scene(
        &mut self,
        depth: &mut UiDepth,
        read: Option<Result<ID3D11Texture2D, DistanceSkip>>,
        input: &DepthInput,
    ) -> Option<Scene> {
        let camera = input.camera?;
        let eye_m = input.eye_m * depth.rendered_eye_scale().share();
        let crossed = crossed_per_unit(lens(&camera), eye_m, camera.far)?;
        let distances = match read {
            Some(Ok(distances)) => distances,
            // The UI depth's read logged its own warning for this frame.
            Some(Err(_)) => return None,
            None => self.read_now(depth)?,
        };
        Some(Scene { distances, crossed })
    }

    /// The frame's distance texture read now, or `None` with a rate-limited warning when it is
    /// missing or unusable.
    fn read_now(&mut self, depth: &mut UiDepth) -> Option<ID3D11Texture2D> {
        match depth.distances() {
            Ok(distances) => Some(distances),
            Err(skip) => {
                warn_every!(
                    self.skip_log,
                    LOG_PERIOD,
                    skip = %skip,
                    "depth map read no distance texture, so the frame's map holds no cell"
                );
                None
            }
        }
    }
}

/// Where a frame drew its UI texture: the columns it moved the UI in each half and the interface
/// area it drew the UI into.
#[derive(Clone, Copy)]
struct UiPlace {
    columns: u16,
    area: InterfaceArea,
}

/// The start of the map of `inputs` in a frame of `size`, whose UI the frame drew at `place` and
/// whose depth offset in use is `offset`, with the present number `present`. The grid covers the
/// UI texture, or the frame in a frame that draws none.
fn map_start(
    inputs: &Inputs,
    place: UiPlace,
    size: BackBuffer,
    offset: f32,
    present: u32,
) -> MapStart<'_> {
    let BackBuffer { width, height, .. } = size;
    let half_width = halves(width, height).map_or(0, |[left, _]| left.width);
    let disparity = ui_disparity(place.columns, half_width);
    let ui = inputs.ui.as_ref().map(|layer| MapUi { layer, disparity, area: place.area });
    let grid = ui.map_or_else(|| Grid::covering(width, height), |drawn| layer_grid(drawn.layer));
    let scene = inputs.scene.as_ref().map_or(MapScene::Flat, |scene| MapScene::Rendered {
        distances: &scene.distances,
        crossed: scene.crossed,
        offset,
    });
    MapStart { grid, present, ui, scene }
}

#[cfg(test)]
mod tests;
