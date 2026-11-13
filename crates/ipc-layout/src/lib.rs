//! Contracts shared across processes: the launch token and viewer header byte layouts, the core DLL's init contract and the perf summary log line.

#![forbid(unsafe_code)]

pub mod core_init;
pub mod launch_token;
pub mod perf_line;
pub mod viewer_header;
