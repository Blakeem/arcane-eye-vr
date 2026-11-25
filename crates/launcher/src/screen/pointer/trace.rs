//! The trace of the pointing hand's ray through the depth the picture shows to the viewer's two
//! eyes: the point where the ray meets what the picture shows, and the game's mouse place there.
//!
//! A point's place for an eye is a place of the picture as shares from its top left. On the screen
//! quad it is where the eye's line through the point meets the quad's plane. In the projection
//! picture it is where the eye view's picture shows the point. A point's disparity is its place's x
//! for the right eye minus its place's x for the left eye. A walked point counts while it has a
//! place for each eye and its place for the left eye lies on the picture. The ray meets the picture
//! at the first counting point whose disparity reaches the depth map's uncrossed disparity under
//! that place. A ray whose walk ends at a counting point with no meeting meets the picture at that
//! end, since the far scene's disparity is one that a point on the ray only approaches. Without a
//! depth map, the projection picture takes the walk's end with the mouse place midway between its
//! two places. A ray that meets a covered scene meets nothing, since the game would give its click
//! to an interface element the picture shows elsewhere.

use ipc_layout::viewer_header::depth_map::InterfaceArea;
use openxr::Vector3f;
use stereo_core::eye::PerEye;
use xr::projection::EyeView;
use xr::quad::ScreenQuad;

use crate::screen::depth::{CellDepth, Depth, DepthView, Kind};
use crate::screen::geometry::{Ray, Shares, crossing, place, view_place};

/// The distance from the ray's start of the first walked point.
const START_M: f32 = 0.05;

/// The distance from the ray's start of the last walked point.
const END_M: f32 = 200.0;

/// The walk's first step.
const FIRST_STEP_M: f32 = 0.01;

/// The length of each step over the step before.
const GROWTH: f32 = 1.02;

/// The halvings between the meeting point and the walked point before it.
const HALVINGS: u32 = 10;

/// Where the ray meets the picture: the point in the LOCAL space and the game's mouse place there,
/// as shares of the game's client area.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Meeting {
    pub(crate) point: Vector3f,
    pub(crate) mouse: Shares,
}

/// The two eyes that see the picture, which give each point its place for each eye.
#[derive(Clone, Copy)]
enum Viewpoints {
    /// The located eye positions in front of the screen quad.
    Screen { quad: ScreenQuad, eyes: PerEye<Vector3f> },
    /// The projection picture's two eye views.
    Projection(PerEye<EyeView>),
}

impl Viewpoints {
    /// The places of `point` for the two eyes, or `None` when the point does not count: an eye
    /// gives it no place or its place for the left eye lies off the picture.
    fn places(self, point: Vector3f) -> Option<PerEye<Shares>> {
        let places = match self {
            Self::Screen { quad, eyes } => {
                let on_plane =
                    |eye| crossing(eye, point, quad.pose).map(|at| place(at, quad.pose, quad.size));
                PerEye { left: on_plane(eyes.left)?, right: on_plane(eyes.right)? }
            }
            Self::Projection(views) => PerEye {
                left: view_place(point, views.left)?,
                right: view_place(point, views.right)?,
            },
        };
        places.left.inside().then_some(places)
    }
}

/// A counting point: its place for the left eye, its disparity and the map's cell at that place.
#[derive(Clone, Copy)]
struct Probe {
    left: Shares,
    disparity: f32,
    cell: CellDepth,
}

impl Probe {
    /// Whether the point lies at or past what the picture shows at its place.
    fn meets(self) -> bool {
        self.disparity >= self.cell.disparity
    }
}

/// Where `ray` meets the picture that `depth` gives the screen quad `quad`, or `None` when no
/// walked point meets it and the walk ends off the picture, when the meeting point shows a covered
/// scene, or when the mouse place of the meeting point lies off the client area.
pub(crate) fn trace(ray: Ray, depth: Depth<'_>, quad: ScreenQuad) -> Option<Meeting> {
    walk_to(ray, depth.map, Viewpoints::Screen { quad, eyes: depth.eyes })
}

/// Where `ray` meets the projection picture of the eye views `views` in the depth of `map`, under
/// the rules of [`trace`].
pub(crate) fn trace_projection(
    ray: Ray,
    map: DepthView<'_>,
    views: PerEye<EyeView>,
) -> Option<Meeting> {
    walk_to(ray, map, Viewpoints::Projection(views))
}

/// Where `ray` meets the far scene of the projection picture of the eye views `views` without a
/// depth map: the walk's last point, with the mouse place midway between its two places. `None`
/// when that point does not count or its mouse place lies off the client area.
pub(crate) fn far_point(ray: Ray, views: PerEye<EyeView>) -> Option<Meeting> {
    let point = ray.at(END_M);
    let places = Viewpoints::Projection(views).places(point)?;
    let (left, right) = (places.left, places.right);
    let mouse = Shares { x: f32::midpoint(left.x, right.x), y: f32::midpoint(left.y, right.y) };
    mouse.inside().then_some(Meeting { point, mouse })
}

/// What the frame of a depth map shows besides its cells: the depth offset by which the core moved
/// the right half's scene, and the interface area it drew the interface into.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Drawn {
    pub(crate) depth_offset: f32,
    pub(crate) area: InterfaceArea,
}

/// The game's mouse place of a thing at the place `left` of the left half in the map's cell `cell`
/// of a frame that `drawn` says how it drew, or `None` off the client area and for a covered
/// scene. The mouse position is a place of the head camera's picture, midway between the thing's
/// places in the two halves. The core moved the right half's scene, and not its interface, right
/// by the depth offset. An interface place is the area place of the interface texel there, whose
/// game place the mouse takes.
pub(crate) fn mouse_place(left: Shares, cell: CellDepth, drawn: Drawn) -> Option<Shares> {
    let middle = left.x + cell.disparity / 2.0;
    let mouse = match cell.kind {
        Kind::Interface => {
            let [x, y] = drawn.area.game_place([middle, left.y]);
            Shares { x, y }
        }
        Kind::Scene => Shares { x: middle - drawn.depth_offset / 2.0, y: left.y },
        Kind::CoveredScene => return None,
    };
    mouse.inside().then_some(mouse)
}

/// Where `ray` meets the picture that `viewpoints` see in the depth of `map`.
fn walk_to(ray: Ray, map: DepthView<'_>, viewpoints: Viewpoints) -> Option<Meeting> {
    let probe_at = |distance: f32| probe(ray.at(distance), map, viewpoints);
    let mut before = None;
    // The walk's end, and no earlier counting point, stands in for the far scene, so a ray whose
    // places only cross the picture in front of what it shows meets nothing.
    let mut end = None;
    for distance in walk() {
        let probed = probe_at(distance);
        if let Some(found) = probed.filter(|probed| probed.meets()) {
            let (meeting_m, found) = refine(before, distance, found, &probe_at);
            return meeting(ray.at(meeting_m), found, drawn_of(map));
        }
        before = Some(distance);
        end = probed.map(|probed| (distance, probed));
    }
    let (end_m, end_probe) = end?;
    meeting(ray.at(end_m), end_probe, drawn_of(map))
}

/// The distances of the walked points from [`START_M`] to [`END_M`].
fn walk() -> impl Iterator<Item = f32> {
    let first = (START_M, FIRST_STEP_M);
    let next = |&(distance, step): &(f32, f32)| {
        (distance < END_M).then(|| ((distance + step).min(END_M), step * GROWTH))
    };
    std::iter::successors(Some(first), next).map(|(distance, _)| distance)
}

/// The probe of `point`, or `None` when the point does not count or the map has no cell under its
/// place for the left eye.
fn probe(point: Vector3f, map: DepthView<'_>, viewpoints: Viewpoints) -> Option<Probe> {
    let places = viewpoints.places(point)?;
    let cell = map.at(places.left)?;
    Some(Probe { left: places.left, disparity: places.right.x - places.left.x, cell })
}

/// The meeting point's distance and probe after [`HALVINGS`] halvings between `before`, the
/// walked point before, and `after`, the first walked point that meets the picture with the probe
/// `found`.
fn refine(
    before: Option<f32>,
    after: f32,
    found: Probe,
    probe_at: &impl Fn(f32) -> Option<Probe>,
) -> (f32, Probe) {
    let Some(mut short) = before else {
        return (after, found);
    };
    let (mut long, mut found) = (after, found);
    for _ in 0..HALVINGS {
        let middle = f32::midpoint(short, long);
        match probe_at(middle).filter(|probed| probed.meets()) {
            Some(probed) => (long, found) = (middle, probed),
            None => short = middle,
        }
    }
    (long, found)
}

fn meeting(point: Vector3f, probe: Probe, drawn: Drawn) -> Option<Meeting> {
    let mouse = mouse_place(probe.left, probe.cell, drawn)?;
    Some(Meeting { point, mouse })
}

const fn drawn_of(map: DepthView<'_>) -> Drawn {
    Drawn { depth_offset: map.depth_offset, area: map.area }
}

#[cfg(test)]
mod tests;
