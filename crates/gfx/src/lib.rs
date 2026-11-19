//! D3D11 resources: the viewer publisher, the layer draw, the nearest read and the region draw, whose
//! shaders the build script compiles.

pub mod layer;
pub mod nearest;
pub mod publish;
mod raw;
pub mod region;
