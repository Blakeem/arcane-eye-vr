//! Health registry, panic hook, log ring layer, allocation guard, rate-limited logging, perf summary aggregation, error chain text, root folder and crash client.

pub mod alloc_guard;
pub mod error_chain;
pub mod health;
mod layer;
mod line;
pub mod logging;
pub mod perf;
pub mod rate;
mod raw;
mod ring;
pub mod root;
mod rotation;
mod writer;

/// A temp folder for test `name` of this crate.
#[cfg(test)]
fn test_dir(name: &str) -> test_support::TestDir {
    test_support::TestDir::in_temp("arcane-eye-diag", name).expect("create the test folder")
}
