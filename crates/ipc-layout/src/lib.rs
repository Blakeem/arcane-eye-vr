//! Contracts shared across processes: the launch token and viewer header byte layouts, the core DLL's init contract, the perf summary and camera log lines and the root folder variable.

#![forbid(unsafe_code)]

pub mod camera_line;
pub mod core_init;
pub mod launch_token;
pub mod log_fields;
pub mod perf_line;
pub mod root_folder;
pub mod viewer_header;
