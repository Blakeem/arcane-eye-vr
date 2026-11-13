//! The core DLL's init export: its name, its signature and its success code.
//!
//! The proxy DLL resolves and calls the export, and the core DLL defines it. Both read these items, so
//! a change to the contract reaches both DLLs.

use core::ffi::CStr;

/// The name the core DLL exports its init function under, which is that function's name.
pub const INIT_EXPORT: &CStr = c"arcane_eye_init";

/// The signature of the init export. `mode` is the launch token's raw mode value.
pub type InitFn = extern "C" fn(mode: u32) -> u32;

/// The code the init export returns on success. Every other code names a failure.
pub const INIT_STARTED: u32 = 0;
