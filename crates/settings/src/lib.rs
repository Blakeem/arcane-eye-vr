//! Settings and dev flag types with their parser, validator and schema.
//!
//! `player.rs` parses the player file `settings.toml` into `Settings`, and `dev.rs` parses the dev file
//! `dev.toml` into `DevFlags`. `file.rs` reads both from the folder its caller names, and writes the
//! table `[screen]` into the player file with the text that `edit.rs` makes. The crate makes no
//! Windows call and reads no environment variable.

#![forbid(unsafe_code)]

pub mod dev;
pub mod edit;
pub mod file;
pub mod flags;
pub mod player;
pub mod rejection;

/// A temp folder for test `name` of this crate.
#[cfg(test)]
fn test_dir(name: &str) -> test_support::TestDir {
    test_support::TestDir::in_temp("arcane-eye-settings", name).expect("create the test folder")
}
