//! The menu panel: a menu of stepper rows, its layout in a panel of pixels, the hit test of a point
//! on the panel and the painter that draws the menu into pixels.
//!
//! The crate gives out pixels and takes points in panel pixels. Its caller shows the pixels, so the
//! crate holds no OpenXR or Direct3D type.

#![forbid(unsafe_code)]

pub mod layout;
pub mod menu;
pub mod painter;
