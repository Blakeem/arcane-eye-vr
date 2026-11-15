//! The launch token: this process creates it before the game starts, and the game reports its status
//! in it.

use std::process;
use std::time::SystemTime;

use ipc_layout::launch_token::{self, LaunchToken, Mode};

pub use crate::raw::{TokenCreateError, TokenMapping};

/// Creates the launch token for a start in `mode`. The token lasts until the mapping is dropped and
/// no game process holds a view of it.
///
/// # Errors
///
/// Returns [`TokenCreateError::Exists`] when a token already exists, including one this process
/// still holds, and [`TokenCreateError::Create`] or [`TokenCreateError::Map`] when Windows cannot
/// create or map it.
pub fn create(mode: Mode) -> Result<TokenMapping, TokenCreateError> {
    let token = LaunchToken::new(mode, process::id(), unix_ms_now());
    TokenMapping::create(launch_token::MAPPING_NAME, &token)
}

/// Milliseconds since the Unix epoch. A clock set before 1970 reads as 0, since the token's
/// timestamp is only a record.
fn unix_ms_now() -> u64 {
    let since_epoch = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default();
    u64::try_from(since_epoch.as_millis()).unwrap_or(u64::MAX)
}
