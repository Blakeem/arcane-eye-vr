//! Device profiles: the data files that bind each controller's inputs to the actions of the `xr`
//! crate's session.
//!
//! `profile.rs` parses one file into a [`profile::DeviceProfile`], and `folder.rs` reads every profile
//! file of a folder. Every device lives in a file, so no code of this crate names one.

#![forbid(unsafe_code)]

pub mod folder;
pub mod profile;
