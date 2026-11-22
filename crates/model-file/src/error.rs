//! The faults that stop a read, each naming where in the file it lies.

use crate::container::{HEADER_LENGTH, MAGIC, VERSION};
use crate::limit::Limit;

/// A fault that stops a read.
#[derive(Debug, thiserror::Error)]
pub enum ReadError {
    #[error("the file holds {length} bytes, fewer than the {HEADER_LENGTH} bytes of a GLB header")]
    ShortFile { length: usize },
    #[error("the file starts with {found:#010x}, not the GLB magic value {MAGIC:#010x}")]
    Magic { found: u32 },
    #[error("the file is GLB version {found}, and the reader reads only version {VERSION}")]
    Version { found: u32 },
    #[error("the GLB header gives a length of {declared} bytes, and the file holds {actual}")]
    LengthPastFile { declared: usize, actual: usize },
    #[error("chunk {chunk} runs past the end of the file")]
    ChunkPastFile { chunk: usize },
    #[error("the first chunk has the type {found:#010x}, not JSON")]
    NoJsonChunk { found: u32 },
    #[error("the JSON chunk does not parse")]
    Json(#[from] serde_json::Error),
    #[error("the file requires the glTF extension {0}")]
    RequiredExtension(String),
    #[error("the file has no scene")]
    NoScene,
    #[error("{kind} index {index} is out of range")]
    Index { kind: IndexKind, index: usize },
    #[error(
        "node {node} is reached twice from the scene, as a root listed twice, a child of two parents or a node of a cycle"
    )]
    NodeCycle { node: usize },
    #[error("buffer {buffer} is outside the GLB, which the reader does not read")]
    ExternalBuffer { buffer: usize },
    #[error("buffer 0 is the binary chunk, and the file has none")]
    NoBinaryChunk,
    #[error("buffer {buffer} is longer than the binary chunk")]
    BufferPastChunk { buffer: usize },
    #[error("buffer view {view} runs past its buffer")]
    ViewPastBuffer { view: usize },
    #[error("accessor {accessor} has no buffer view, which the reader does not read")]
    NoBufferView { accessor: usize },
    #[error("accessor {accessor} is sparse, which the reader does not read")]
    Sparse { accessor: usize },
    #[error(
        "accessor {accessor} has the component type {component_type}, which the reader does not read there"
    )]
    ComponentType { accessor: usize, component_type: u32 },
    #[error("accessor {accessor} has the type {kind}, which the reader does not read there")]
    AccessorType { accessor: usize, kind: String },
    #[error(
        "accessor {accessor} has elements of {size} bytes, more than its buffer view's stride of {stride}"
    )]
    Stride { accessor: usize, size: usize, stride: usize },
    #[error("accessor {accessor} runs past its buffer view")]
    AccessorPastView { accessor: usize },
    #[error("primitive {primitive} of mesh {mesh}: {fault}")]
    Primitive { mesh: usize, primitive: usize, fault: PrimitiveFault },
    #[error("the file passes the limit of {0}")]
    Limit(#[from] Limit),
}

/// The array of the glTF JSON that an index points into.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IndexKind {
    Scene,
    Node,
    Mesh,
    Accessor,
    BufferView,
    Buffer,
    Material,
    Texture,
    Image,
}

impl IndexKind {
    /// The array's name in an error message.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Scene => "scene",
            Self::Node => "node",
            Self::Mesh => "mesh",
            Self::Accessor => "accessor",
            Self::BufferView => "buffer view",
            Self::Buffer => "buffer",
            Self::Material => "material",
            Self::Texture => "texture",
            Self::Image => "image",
        }
    }
}

impl std::fmt::Display for IndexKind {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// A fault of one primitive's vertex data.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum PrimitiveFault {
    #[error("an attribute has another count than the positions")]
    AttributeCount,
    #[error("the index {index} is not below the vertex count")]
    VertexIndex { index: u32 },
    #[error("the index count is no multiple of 3")]
    PartialTriangle,
    #[error("the triangle strip or fan holds fewer than 3 indices")]
    TooFewIndices,
}

/// The item at `index` of the JSON array `items`.
pub(crate) fn item<T>(items: &[T], kind: IndexKind, index: usize) -> Result<&T, ReadError> {
    items.get(index).ok_or(ReadError::Index { kind, index })
}
