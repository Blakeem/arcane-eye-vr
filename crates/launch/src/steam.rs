//! The Steam start: opens the game's Steam URL. The player then clicks Play in the Larian launcher.

use tracing::info;

use crate::raw;

pub use crate::raw::OpenUrlError;

/// The Steam URL that starts the game.
const STEAM_URL: &str = concat!("steam://rungameid/", steam_app_id!());

/// Asks Steam to start the game.
///
/// # Errors
///
/// Returns [`OpenUrlError`] when Windows cannot open the Steam URL.
pub fn start() -> Result<(), OpenUrlError> {
    raw::open_url(STEAM_URL)?;
    info!(url = STEAM_URL, "Steam URL opened");
    Ok(())
}
