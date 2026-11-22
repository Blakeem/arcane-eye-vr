//! The OpenXR side of the headset screen and the immersive mode: the runtime load, the session, its
//! frame loop, its swapchains, its panels, its quad and projection layers, the head's and the eye
//! views' read, the controllers' input through device profiles and the render models of the
//! devices the user holds with their blended projection layer. It knows no game type and names no
//! device.
//!
//! The pure decisions are in `adapter.rs`, `extensions.rs`, `format.rs`, `input.rs`, `layers.rs`,
//! `manifest.rs`, `model/decide.rs`, `quad.rs` and `state.rs`, in `located` and `once_per_time`
//! of `head.rs` and in `located` and `recommended_size` of `projection.rs`. `raw.rs` and
//! `raw/render_model.rs` hold every unsafe call.

pub mod adapter;
mod controls;
pub mod error;
mod extensions;
pub mod format;
pub mod head;
pub mod input;
mod layers;
pub mod manifest;
pub mod model;
pub mod panel;
pub mod projection;
pub mod quad;
mod raw;
pub mod runtime;
pub mod session;
mod state;
pub mod swapchain;
