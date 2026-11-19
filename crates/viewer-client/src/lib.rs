//! The reading side of the viewer link that the core publishes the back buffer through.
//!
//! The debug viewer draws the frames in a window, the launch app's headset screen draws them in a
//! headset, and a test runner reads them back to save them. This crate owns the header view, the
//! heartbeat thread, the head pose write, the device on the core's adapter, the open of the shared
//! texture, its private copy, the copy out of it or one region of it under its keyed mutex and the
//! frame read. The `raw` module and its submodules hold every unsafe Windows call.

pub mod frame;
pub mod header;
pub mod heartbeat;
mod raw;
pub mod region;

#[cfg(any(test, feature = "fixture"))]
pub use raw::fixture;
pub use raw::{Copied, Device, GpuError, HeaderView, HeaderViewError, SharedTexture};
