//! The core DLL's file name and its init export: the export's name, signature and success code.
//!
//! The proxy DLL loads the core by its file name, then resolves and calls the export, which the core
//! DLL defines. Each reads these items, so a change to the contract reaches each of them.

use core::ffi::CStr;

/// The core DLL's file name. It sits in the game's bin folder next to the proxy.
pub const CORE_DLL: &str = "arcane_eye_core.dll";

/// The name the core DLL exports its init function under, which is that function's name.
pub const INIT_EXPORT: &CStr = c"arcane_eye_init";

/// The signature of the init export. `mode` is the launch token's raw mode value.
pub type InitFn = extern "system" fn(mode: u32) -> u32;

/// The code the init export returns on success. Every other code names a failure.
pub const INIT_STARTED: u32 = 0;
