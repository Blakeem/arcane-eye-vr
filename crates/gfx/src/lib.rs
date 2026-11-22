//! D3D11 resources: the viewer publisher, the layer draw, the nearest read, the pointer read, the
//! depth map, the region draw and the mesh draw, whose shaders the build script compiles. Also the
//! pixel canvas, which GDI draws text into.

pub mod canvas;
pub mod depth_map;
pub mod layer;
pub mod mesh;
pub mod nearest;
pub mod pointer;
pub mod publish;
mod raw;
mod readback;
pub mod region;
