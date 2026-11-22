//! Every way the headset session fails. The player reads each message in a message box, so it names
//! the cause and the fix. A game that runs keeps running after each of them. The screen mode and
//! the immersive mode share each failure, so a fix gets the `--mode` name of the mode the player
//! started from its caller.

use std::io;

use diag::error_chain::chain;
use gfx::region::RegionError;
use viewer_client::GpuError;
use xr::runtime::{AdapterError, ConnectError, LoadError};
use xr::session::SessionError;

use super::FrameError;
use super::picture::Unshowable;

/// The step before the restart of a failure in SteamVR's part of the session.
const RESTART_STEP: &str = "Restart SteamVR";

/// The step before the restart of a failure of the graphics adapter.
const DRIVER_STEP: &str = "Update the graphics driver";

/// Why the headset session stopped before the game or the session ended. Each message names the
/// cause, and [`ScreenFailure::fix`] gives the fix that follows it.
#[derive(Debug, thiserror::Error)]
pub(crate) enum ScreenFailure {
    #[error("Arcane Eye VR could not start its heartbeat thread ({0}).")]
    Heartbeat(#[source] io::Error),
    #[error("Arcane Eye VR could not load SteamVR's OpenXR runtime ({}).", chain(.0))]
    RuntimeLoad(#[from] LoadError),
    #[error("SteamVR finds no headset ({0}).")]
    NoHeadset(#[source] ConnectError),
    #[error("SteamVR gave Arcane Eye VR no OpenXR instance ({0}).")]
    Connect(#[source] ConnectError),
    #[error("Arcane Eye VR found no graphics adapter for the headset ({}).", chain(.0))]
    Adapter(#[from] AdapterError),
    #[error("Arcane Eye VR could not use the headset's graphics adapter ({0}).")]
    Device(#[source] GpuError),
    #[error("Arcane Eye VR could not create its shaders ({0}).")]
    Draw(#[from] RegionError),
    #[error("SteamVR could not open a headset session ({0}).")]
    Session(#[from] SessionError),
    #[error("The headset session failed ({0}).")]
    Frame(#[source] FrameError),
    #[error("Arcane Eye VR cannot show the game's frames in the headset ({0}).")]
    Unshowable(#[source] Unshowable),
}

/// What the player does before Arcane Eye VR starts again.
#[derive(Clone, Copy)]
enum Fix {
    /// Only the start.
    Start,
    /// The step, then the start in the mode the player started.
    Then(&'static str),
}

impl ScreenFailure {
    /// The fix of the failure, for the player who started the mode whose `--mode` name is `mode`.
    pub(crate) fn fix(&self, mode: &str) -> String {
        match self.step() {
            Fix::Start => "Start Arcane Eye VR again.".to_owned(),
            Fix::Then(step) => {
                format!("{step}, then start Arcane Eye VR with --mode {mode} again.")
            }
        }
    }

    const fn step(&self) -> Fix {
        match self {
            Self::Heartbeat(_) => Fix::Start,
            Self::RuntimeLoad(_) => Fix::Then("Reinstall SteamVR"),
            Self::NoHeadset(_) => Fix::Then("Connect the headset"),
            Self::Connect(_) | Self::Frame(_) => Fix::Then(RESTART_STEP),
            Self::Adapter(_) | Self::Device(_) | Self::Draw(_) => Fix::Then(DRIVER_STEP),
            Self::Session(cause) => Fix::Then(session_step(cause)),
            Self::Unshowable(cause) => Fix::Then(unshowable_step(*cause)),
        }
    }
}

/// The step of a session that did not open. A SteamVR restart does not raise the adapter's feature
/// level.
const fn session_step(cause: &SessionError) -> &'static str {
    match cause {
        SessionError::FeatureLevel { .. } => DRIVER_STEP,
        SessionError::Create(_)
        | SessionError::Formats(_)
        | SessionError::NoFormat(_)
        | SessionError::Space(_) => RESTART_STEP,
    }
}

/// The step of frames the session cannot show.
const fn unshowable_step(cause: Unshowable) -> &'static str {
    match cause {
        Unshowable::OtherAdapter => {
            "Quit the game, set bg3_dx11.exe to the headset's graphics adapter in the Windows \
             graphics settings"
        }
        Unshowable::UnsupportedFormat(_) => "If HDR is on in the game, switch it off",
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
    use std::io;

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
             below the runtime's minimum 0xb000)."
        );
        assert_eq!(
            low.fix("screen"),
            "Update the graphics driver, then start Arcane Eye VR with --mode screen again."
        );
        let create = ScreenFailure::from(SessionError::Create(sys::Result::ERROR_RUNTIME_FAILURE));
        assert_eq!(
            create.fix("screen"),
            "Restart SteamVR, then start Arcane Eye VR with --mode screen again."
        );
    }

    #[test]
    fn each_fix_that_starts_arcane_eye_vr_again_names_the_mode_the_player_started() {
        let missing = ConnectError::NoHeadset(sys::Result::ERROR_FORM_FACTOR_UNAVAILABLE);
        let failure = ScreenFailure::from(missing);
        assert_eq!(
            failure.fix("screen"),
            "Connect the headset, then start Arcane Eye VR with --mode screen again."
        );
        assert_eq!(
            failure.fix("vr"),
            "Connect the headset, then start Arcane Eye VR with --mode vr again."
        );
        let heartbeat = ScreenFailure::Heartbeat(io::Error::other("no thread"));
        assert_eq!(heartbeat.fix("vr"), "Start Arcane Eye VR again.");
    }

    #[test]
    fn frames_the_screen_cannot_show_end_it_with_their_cause_and_fix() {
        let format = ScreenFailure::from(FrameError::Unshowable(Unshowable::UnsupportedFormat(10)));
        assert!(matches!(format, ScreenFailure::Unshowable(_)), "{format:?}");
        assert_eq!(
            format.to_string(),
            "Arcane Eye VR cannot show the game's frames in the headset (the game's frames have the \
             DXGI format 10, which the screen does not show)."
        );
        assert_eq!(
            format.fix("screen"),
            "If HDR is on in the game, switch it off, then start Arcane Eye VR with --mode screen \
             again."
        );
        let adapter = ScreenFailure::from(FrameError::Unshowable(Unshowable::OtherAdapter));
        assert_eq!(
            adapter.fix("screen"),
            "Quit the game, set bg3_dx11.exe to the headset's graphics adapter in the Windows \
             graphics settings, then start Arcane Eye VR with --mode screen again."
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
        let instance =
            ScreenFailure::from(ConnectError::Instance(sys::Result::ERROR_RUNTIME_FAILURE));
        assert!(matches!(instance, ScreenFailure::Connect(_)), "{instance:?}");
        assert_eq!(
            instance.fix("screen"),
            "Restart SteamVR, then start Arcane Eye VR with --mode screen again."
        );
    }
}
