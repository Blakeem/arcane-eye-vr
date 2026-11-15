//! Settings and dev flag types with their parser, validator and schema.
//!
//! `player.rs` parses the player file `settings.toml` into `Settings`, and `dev.rs` parses the dev file
//! `dev.toml` into `DevFlags`. `file.rs` reads both from the folder its caller names, and `watch.rs`
//! tells which of the two changed since its check before. The crate makes no Windows call and reads no
//! environment variable.

#![forbid(unsafe_code)]

pub mod dev;
pub mod file;
pub mod flags;
pub mod player;
pub mod rejection;
pub mod watch;

/// A temp folder for test `name` of this crate.
#[cfg(test)]
fn test_dir(name: &str) -> test_support::TestDir {
    test_support::TestDir::in_temp("arcane-eye-settings", name).expect("create the test folder")
}
