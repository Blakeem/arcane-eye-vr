//! The OpenXR side of the headset screen: the runtime load, the session, its frame loop, its
//! swapchains and its quad layers. It knows no game type.
//!
//! The pure decisions are in `adapter.rs`, `format.rs`, `manifest.rs`, `quad.rs` and `state.rs`.
//! `raw.rs` holds every unsafe call.

pub mod adapter;
pub mod error;
pub mod format;
pub mod manifest;
pub mod quad;
mod raw;
pub mod runtime;
pub mod session;
mod state;
pub mod swapchain;
