//! Every way the headset screen fails. The player reads each message in a message box, so it names
//! the cause and the fix. The game keeps running after each of them.

use std::io;

use diag::error_chain::chain;
use gfx::region::RegionError;
use viewer_client::{GpuError, HeaderViewError};
use xr::runtime::{AdapterError, ConnectError, LoadError};
use xr::session::SessionError;

use super::FrameError;
use super::picture::Unshowable;

/// The fix of a failure in SteamVR's part of the screen.
const RESTART_FIX: &str = "Restart SteamVR, then start Arcane Eye VR with --mode screen again.";

/// The fix of a failure of the graphics adapter.
const DRIVER_FIX: &str =
    "Update the graphics driver, then start Arcane Eye VR with --mode screen again.";

/// Why the headset screen stopped before the game or the session ended.
#[derive(Debug, thiserror::Error)]
pub(crate) enum ScreenFailure {
    #[error(
        "Arcane Eye VR could not open the game's frames ({}). Quit the game, then start Arcane \
         Eye VR again.",
        chain(.0)
    )]
    Link(#[from] HeaderViewError),
    #[error("Arcane Eye VR could not start its heartbeat thread ({0}). Start Arcane Eye VR again.")]
    Heartbeat(#[source] io::Error),
    #[error(
        "Arcane Eye VR could not load SteamVR's OpenXR runtime ({}). Reinstall SteamVR, then \
         start Arcane Eye VR with --mode screen again.",
        chain(.0)
    )]
    RuntimeLoad(#[from] LoadError),
    #[error(
        "SteamVR finds no headset ({0}). Connect the headset, then start Arcane Eye VR with \
         --mode screen again."
    )]
    NoHeadset(#[source] ConnectError),
    #[error("SteamVR gave Arcane Eye VR no OpenXR instance ({0}). {RESTART_FIX}")]
    Connect(#[source] ConnectError),
    #[error(
        "Arcane Eye VR found no graphics adapter for the headset ({}). {DRIVER_FIX}",
        chain(.0)
    )]
    Adapter(#[from] AdapterError),
    #[error("Arcane Eye VR could not use the headset's graphics adapter ({0}). {DRIVER_FIX}")]
    Device(#[source] GpuError),
    #[error("Arcane Eye VR could not create its shaders ({0}). {DRIVER_FIX}")]
    Draw(#[from] RegionError),
    #[error(
        "SteamVR could not open a headset session ({0}). {fix}",
        fix = session_fix(.0)
    )]
    Session(#[from] SessionError),
    #[error("The headset session failed ({0}). {RESTART_FIX}")]
    Frame(#[source] FrameError),
    #[error(
        "Arcane Eye VR cannot show the game's frames in the headset ({0}). {fix}",
        fix = unshowable_fix(*.0)
    )]
    Unshowable(#[source] Unshowable),
}

/// The fix of a session that did not open. A SteamVR restart does not raise the adapter's feature
/// level.
const fn session_fix(cause: &SessionError) -> &'static str {
    match cause {
        SessionError::FeatureLevel { .. } => DRIVER_FIX,
        SessionError::Create(_)
        | SessionError::Formats(_)
        | SessionError::NoFormat(_)
        | SessionError::Space(_) => RESTART_FIX,
    }
}

/// The fix of frames the screen cannot show.
const fn unshowable_fix(cause: Unshowable) -> &'static str {
    match cause {
        Unshowable::OtherAdapter => {
            "Quit the game, set bg3_dx11.exe to the headset's graphics adapter in the Windows \
             graphics settings, then start Arcane Eye VR with --mode screen again."
        }
        Unshowable::UnsupportedFormat(_) => {
            "If HDR is on in the game, switch it off, then start Arcane Eye VR with --mode screen \
             again."
        }
    }
}

impl From<FrameError> for ScreenFailure {
    fn from(error: FrameError) -> Self {
        match error {
            FrameError::Unshowable(cause) => Self::Unshowable(cause),
            FrameError::Xr(_) | FrameError::Eyes(_) => Self::Frame(error),
        }
    }
}

impl From<ConnectError> for ScreenFailure {
    fn from(error: ConnectError) -> Self {
        match error {
            ConnectError::NoHeadset(_) => Self::NoHeadset(error),
            ConnectError::Instance(_) | ConnectError::Requirements(_) => Self::Connect(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ScreenFailure;
    use crate::screen::FrameError;
    use crate::screen::eyes::EyesError;
    use crate::screen::picture::Unshowable;
    use openxr::sys;
    use xr::runtime::ConnectError;
    use xr::session::SessionError;

    #[test]
    fn a_device_below_the_runtime_feature_level_names_the_driver_fix() {
        let low =
            ScreenFailure::from(SessionError::FeatureLevel { device: 0x9100, required: 0xB000 });
        assert_eq!(
            low.to_string(),
            "SteamVR could not open a headset session (the D3D11 device's feature level 0x9100 is \
             below the runtime's minimum 0xb000). Update the graphics driver, then start Arcane \
             Eye VR with --mode screen again."
        );
        let create = ScreenFailure::from(SessionError::Create(sys::Result::ERROR_RUNTIME_FAILURE));
        assert!(
            create
                .to_string()
                .ends_with("Restart SteamVR, then start Arcane Eye VR with --mode screen again.")
        );
    }

    #[test]
    fn frames_the_screen_cannot_show_end_it_with_their_cause_and_fix() {
        let format = ScreenFailure::from(FrameError::Unshowable(Unshowable::UnsupportedFormat(10)));
        assert!(matches!(format, ScreenFailure::Unshowable(_)), "{format:?}");
        assert_eq!(
            format.to_string(),
            "Arcane Eye VR cannot show the game's frames in the headset (the game's frames have the \
             DXGI format 10, which the screen does not show). If HDR is on in the game, switch it \
             off, then start Arcane Eye VR with --mode screen again."
        );
        let adapter = ScreenFailure::from(FrameError::Unshowable(Unshowable::OtherAdapter));
        assert_eq!(
            adapter.to_string(),
            "Arcane Eye VR cannot show the game's frames in the headset (the game draws on another \
             graphics adapter than the headset's). Quit the game, set bg3_dx11.exe to the \
             headset's graphics adapter in the Windows graphics settings, then start Arcane Eye VR \
             with --mode screen again."
        );
        let session = ScreenFailure::from(FrameError::Eyes(EyesError::NoImage { index: 3 }));
        assert!(matches!(session, ScreenFailure::Frame(_)), "{session:?}");
    }

    #[test]
    fn a_missing_headset_is_its_own_failure_with_its_fix() {
        let missing = ConnectError::NoHeadset(sys::Result::ERROR_FORM_FACTOR_UNAVAILABLE);
        let failure = ScreenFailure::from(missing);
        assert!(matches!(failure, ScreenFailure::NoHeadset(_)), "{failure:?}");
        let message = failure.to_string();
        assert!(
            message.starts_with("SteamVR finds no headset (the runtime has no head"),
            "{message}"
        );
        assert!(
            message.ends_with(
                "Connect the headset, then start Arcane Eye VR with --mode screen again."
            )
        );
        let instance =
            ScreenFailure::from(ConnectError::Instance(sys::Result::ERROR_RUNTIME_FAILURE));
        assert!(matches!(instance, ScreenFailure::Connect(_)), "{instance:?}");
        assert!(
            instance
                .to_string()
                .ends_with("Restart SteamVR, then start Arcane Eye VR with --mode screen again.")
        );
    }
}
