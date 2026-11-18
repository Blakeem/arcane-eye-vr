//! Contracts shared across processes: the launch token and viewer header byte layouts, the core DLL's init contract, the perf summary, camera and game state log lines, the root folder variable and the settings reload event's name.

#![forbid(unsafe_code)]

pub mod camera_line;
pub mod core_init;
pub mod game_state_line;
pub mod launch_token;
pub mod log_fields;
pub mod perf_line;
pub mod root_folder;
pub mod settings_reload;
pub mod viewer_header;
