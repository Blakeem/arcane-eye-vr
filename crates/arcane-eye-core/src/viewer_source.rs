//! The debug viewer's game source: the game texture the viewer link publishes in place of the back
//! buffer, as the dev flag `viewer.source` names it.

use game_bindings::signatures::G_PIPELINE;
use game_bindings::texture_table::TextureKey;
use settings::dev::ViewerSource;
use tracing::{info, warn};

use crate::config::Config;
use crate::game_source::GameSource;
use crate::install::GameCode;

/// The key of the game texture that `flag` names, or `None` for the back buffer.
pub(crate) const fn key_of(flag: ViewerSource) -> Option<TextureKey> {
    match flag {
        ViewerSource::BackBuffer => None,
        ViewerSource::Scene => Some(TextureKey::SCENE),
        ViewerSource::Ui => Some(TextureKey::UI),
    }
}

/// The game source that `config` names, found in `code`, the game exe's code. Returns `None` for
/// the back buffer, and with a warning when the pipeline global is not found.
pub(crate) fn install(code: &GameCode, config: Config) -> Option<GameSource> {
    let flag = config.dev.viewer.source;
    let key = key_of(flag)?;
    match code.locate(&G_PIPELINE) {
        Ok(pipeline) => {
            info!(source = flag.as_str(), "viewer source installed");
            Some(GameSource::new(key, pipeline))
        }
        Err(error) => {
            warn!(error = %error, "viewer source not installed, so the viewer gets the back buffer");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use game_bindings::texture_table::TextureKey;
    use settings::dev::ViewerSource;

    use super::key_of;

    #[test]
    fn each_flag_value_names_its_key_and_the_back_buffer_names_none() {
        assert_eq!(key_of(ViewerSource::BackBuffer), None);
        assert_eq!(key_of(ViewerSource::Scene), Some(TextureKey::SCENE));
        assert_eq!(key_of(ViewerSource::Ui), Some(TextureKey::UI));
    }
}
