//! The OpenXR side of the headset screen: the runtime load, the session, its frame loop, its
//! swapchains, its panels, its quad layers and the controllers' input through device profiles. It
//! knows no game type and names no device.
//!
//! The pure decisions are in `adapter.rs`, `extensions.rs`, `format.rs`, `input.rs`, `layers.rs`,
//! `manifest.rs`, `quad.rs` and `state.rs`. `raw.rs` holds every unsafe call.

pub mod adapter;
mod controls;
pub mod error;
mod extensions;
pub mod format;
pub mod input;
mod layers;
pub mod manifest;
pub mod panel;
pub mod quad;
mod raw;
pub mod runtime;
pub mod session;
mod state;
pub mod swapchain;
