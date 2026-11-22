//! Meshes: each primitive of triangles, a triangle strip or a triangle fan read into one vertex array
//! per attribute and the `u32` indices of a triangle list.

use crate::accessor::Elements;
use crate::buffer::Source;
use crate::document;
use crate::error::{IndexKind, PrimitiveFault, ReadError, item};
use crate::limit::Budget;
use crate::model::{Mesh, Primitive};

/// The glTF primitive mode of a triangle list, which is also the default mode.
const TRIANGLES: u32 = 4;
const TRIANGLE_STRIP: u32 = 5;
const TRIANGLE_FAN: u32 = 6;

/// The fewest indices of a triangle strip or fan, the corners of its first triangle.
const MIN_STRIP_INDICES: usize = 3;

/// How the indices of a primitive form triangles.
#[derive(Clone, Copy, Debug)]
enum Topology {
    List,
    Strip,
    Fan,
}

impl Topology {
    /// The topology of the glTF `mode`, or `None` for a mode of points or lines.
    fn of_mode(mode: Option<u32>) -> Option<Self> {
        match mode.unwrap_or(TRIANGLES) {
            TRIANGLES => Some(Self::List),
            TRIANGLE_STRIP => Some(Self::Strip),
            TRIANGLE_FAN => Some(Self::Fan),
            _ => None,
        }
    }
}

/// The face normal of a triangle with no area. Such a triangle draws no pixel, so any unit normal serves.
const DEGENERATE_NORMAL: [f32; 3] = [0.0, 0.0, 1.0];

/// Every mesh of the file, at its index in the file.
pub(crate) fn meshes(source: &Source<'_>, budget: &mut Budget) -> Result<Vec<Mesh>, ReadError> {
    let mut meshes = Vec::with_capacity(source.document.meshes.len());
    for (mesh_index, mesh) in source.document.meshes.iter().enumerate() {
        let mut primitives = Vec::new();
        for (primitive_index, primitive) in mesh.primitives.iter().enumerate() {
            let fault = |fault| ReadError::Primitive {
                mesh: mesh_index,
                primitive: primitive_index,
                fault,
            };
            if let Some(read) = read_primitive(source, primitive, budget, &fault)? {
                primitives.push(read);
            }
        }
        meshes.push(Mesh { primitives });
    }
    Ok(meshes)
}

/// The primitive's triangles, or `None` for a primitive of points or lines or without positions.
fn read_primitive(
    source: &Source<'_>,
    primitive: &document::Primitive,
    budget: &mut Budget,
    fault: &impl Fn(PrimitiveFault) -> ReadError,
) -> Result<Option<Primitive>, ReadError> {
    let attributes = &primitive.attributes;
    let Some(position_index) = attributes.position else { return Ok(None) };
    let Some(topology) = Topology::of_mode(primitive.mode) else { return Ok(None) };
    if let Some(material) = primitive.material {
        item(&source.document.materials, IndexKind::Material, material)?;
    }

    let positions = Elements::open(source, position_index)?;
    budget.take_vertices(positions.count)?;
    let positions = positions.vec3s()?;
    let normals = match attributes.normal {
        Some(index) => Some(attribute(source, index, positions.len(), fault)?.vec3s()?),
        None => None,
    };
    let tex_coords = match attributes.tex_coord {
        Some(index) => attribute(source, index, positions.len(), fault)?.tex_coords()?,
        None => vec![[0.0; 2]; positions.len()],
    };
    let indices = indices(source, primitive.indices, positions.len(), budget)?;
    let indices = triangle_list(topology, indices, budget, fault)?;
    check_indices(&indices, positions.len(), fault)?;

    let Some(normals) = normals else {
        budget.take_vertices(indices.len())?;
        let flat = flat(&positions, &tex_coords, &indices, fault)?;
        return Ok(Some(Primitive { material: primitive.material, ..flat }));
    };
    Ok(Some(Primitive { positions, normals, tex_coords, indices, material: primitive.material }))
}

/// The accessor at `index` of an attribute beside the positions, checked to hold `count` elements.
fn attribute<'file>(
    source: &Source<'file>,
    index: usize,
    count: usize,
    fault: &impl Fn(PrimitiveFault) -> ReadError,
) -> Result<Elements<'file>, ReadError> {
    let elements = Elements::open(source, index)?;
    if elements.count != count {
        return Err(fault(PrimitiveFault::AttributeCount));
    }
    Ok(elements)
}

/// The primitive's indices, or 0, 1, 2 and so on for each vertex when the file gives none.
fn indices(
    source: &Source<'_>,
    accessor: Option<usize>,
    vertex_count: usize,
    budget: &mut Budget,
) -> Result<Vec<u32>, ReadError> {
    let Some(accessor) = accessor else {
        budget.take_indices(vertex_count)?;
        return Ok(sequence(vertex_count));
    };
    let elements = Elements::open(source, accessor)?;
    budget.take_indices(elements.count)?;
    elements.indices()
}

/// The triangle list of the `indices` of `topology`, each triangle of a strip or fan with the corners
/// in the order the glTF specification gives, so every triangle keeps the winding of the first.
fn triangle_list(
    topology: Topology,
    indices: Vec<u32>,
    budget: &mut Budget,
    fault: &impl Fn(PrimitiveFault) -> ReadError,
) -> Result<Vec<u32>, ReadError> {
    let corners: fn(&[u32]) -> Vec<u32> = match topology {
        Topology::List => return Ok(indices),
        Topology::Strip => strip_corners,
        Topology::Fan => fan_corners,
    };
    if indices.len() < MIN_STRIP_INDICES {
        return Err(fault(PrimitiveFault::TooFewIndices));
    }
    // The first triangle takes three indices and each later index adds one triangle.
    let triangle_count = indices.len().saturating_sub(MIN_STRIP_INDICES).saturating_add(1);
    budget.take_indices(triangle_count.saturating_mul(3))?;
    Ok(corners(&indices))
}

/// The corners of each triangle `i` of a strip, {v(i), v(i + 1 + i % 2), v(i + 2 - i % 2)}.
fn strip_corners(indices: &[u32]) -> Vec<u32> {
    let triangle = |(i, &[a, b, c]): (usize, &[u32; 3])| {
        if i.is_multiple_of(2) { [a, b, c] } else { [a, c, b] }
    };
    indices.array_windows::<3>().enumerate().flat_map(triangle).collect()
}

/// The corners of each triangle `i` of a fan, {v(i + 1), v(i + 2), v(0)}.
fn fan_corners(indices: &[u32]) -> Vec<u32> {
    let Some((&hub, rim)) = indices.split_first() else { return Vec::new() };
    rim.array_windows::<2>().flat_map(|&[b, c]| [b, c, hub]).collect()
}

fn check_indices(
    indices: &[u32],
    vertex_count: usize,
    fault: &impl Fn(PrimitiveFault) -> ReadError,
) -> Result<(), ReadError> {
    if !indices.len().is_multiple_of(3) {
        return Err(fault(PrimitiveFault::PartialTriangle));
    }
    match indices.iter().find(|&&index| slot(index) >= vertex_count) {
        Some(&index) => Err(fault(PrimitiveFault::VertexIndex { index })),
        None => Ok(()),
    }
}

/// The primitive with its vertices repeated for each triangle corner, so each carries its triangle's
/// face normal. glTF requires flat normals where a primitive has none.
fn flat(
    positions: &[[f32; 3]],
    tex_coords: &[[f32; 2]],
    indices: &[u32],
    fault: &impl Fn(PrimitiveFault) -> ReadError,
) -> Result<Primitive, ReadError> {
    let mut read = Primitive {
        positions: Vec::with_capacity(indices.len()),
        normals: Vec::with_capacity(indices.len()),
        tex_coords: Vec::with_capacity(indices.len()),
        indices: sequence(indices.len()),
        material: None,
    };
    let (triangles, _) = indices.as_chunks::<3>();
    for &[a, b, c] in triangles {
        let vertex = |index| corner(positions, tex_coords, index, fault);
        let corners = [vertex(a)?, vertex(b)?, vertex(c)?];
        let normal = face_normal(corners.map(|(position, _)| position));
        for (position, tex_coord) in corners {
            read.positions.push(position);
            read.normals.push(normal);
            read.tex_coords.push(tex_coord);
        }
    }
    Ok(read)
}

/// The position and texture coordinate of the vertex `index`.
fn corner(
    positions: &[[f32; 3]],
    tex_coords: &[[f32; 2]],
    index: u32,
    fault: &impl Fn(PrimitiveFault) -> ReadError,
) -> Result<([f32; 3], [f32; 2]), ReadError> {
    let missing = || fault(PrimitiveFault::VertexIndex { index });
    let position = positions.get(slot(index)).ok_or_else(missing)?;
    let tex_coord = tex_coords.get(slot(index)).ok_or_else(missing)?;
    Ok((*position, *tex_coord))
}

/// The unit normal of the counterclockwise triangle of the three `corners`.
fn face_normal(corners: [[f32; 3]; 3]) -> [f32; 3] {
    let [[ax, ay, az], [bx, by, bz], [cx, cy, cz]] = corners;
    let [ux, uy, uz] = [bx - ax, by - ay, bz - az];
    let [vx, vy, vz] = [cx - ax, cy - ay, cz - az];
    let cross = [uy * vz - uz * vy, uz * vx - ux * vz, ux * vy - uy * vx];
    let length = cross.iter().map(|component| component * component).sum::<f32>().sqrt();
    if !length.is_normal() {
        return DEGENERATE_NORMAL;
    }
    cross.map(|component| component / length)
}

/// The indices 0, 1, 2 and so on, `count` of them.
fn sequence(count: usize) -> Vec<u32> {
    (0..=u32::MAX).take(count).collect()
}

/// The slot of `index` in a vertex array. An index past `usize` is past every array.
fn slot(index: u32) -> usize {
    usize::try_from(index).unwrap_or(usize::MAX)
}
