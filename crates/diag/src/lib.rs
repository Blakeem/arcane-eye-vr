//! Health registry, panic hook, log ring layer, allocation guard, rate-limited logging, perf summary aggregation, error chain text and crash client.

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
mod rotation;
#[cfg(test)]
mod test_dir;
mod writer;
