//! Byte layouts shared across processes, such as the launch token and the viewer header, and the core DLL's init contract.

#![forbid(unsafe_code)]

pub mod core_init;
pub mod launch_token;
pub mod viewer_header;
