//! D3D11 resources: the viewer publisher, the layer draw, the nearest read, the pointer read and the
//! region draw, whose shaders the build script compiles.

pub mod layer;
pub mod nearest;
pub mod pointer;
pub mod publish;
mod raw;
mod readback;
pub mod region;
