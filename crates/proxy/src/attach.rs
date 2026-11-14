//! The proxy's decisions as pure functions: what process attach does, and where the core DLL lives.

use std::path::{Path, PathBuf};

use ipc_layout::core_init::{CORE_DLL, INIT_STARTED};
use ipc_layout::launch_token::{LaunchToken, TokenError};

/// Which game executable loaded the proxy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Exe {
    /// `bg3_dx11.exe`, the DirectX 11 renderer the mod hooks.
    Dx11,
    /// `bg3.exe`, the Vulkan renderer the mod does not support.
    Vulkan,
    /// Any other process that loads a `version.dll` from the game folder.
    Other,
}

/// What process attach does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Action {
    StartNothing,
    ReportWrongRenderer,
    /// Writes the game process id and starts the loader thread, which passes `mode` to the core.
    LoadCore {
        mode: u32,
    },
}

/// The executable named by `exe_path`. Windows file names ignore case.
pub(crate) fn exe(exe_path: &Path) -> Exe {
    let file_name = exe_path.file_name().and_then(|name| name.to_str()).unwrap_or_default();
    if file_name.eq_ignore_ascii_case("bg3_dx11.exe") {
        Exe::Dx11
    } else if file_name.eq_ignore_ascii_case("bg3.exe") {
        Exe::Vulkan
    } else {
        Exe::Other
    }
}

/// The action for `exe` and the decoded launch token. An invalid token starts nothing.
pub(crate) fn action(exe: Exe, token: Result<LaunchToken, TokenError>) -> Action {
    let Ok(token) = token else {
        return Action::StartNothing;
    };
    match exe {
        Exe::Dx11 => Action::LoadCore { mode: token.raw_mode() },
        Exe::Vulkan => Action::ReportWrongRenderer,
        Exe::Other => Action::StartNothing,
    }
}

/// The core DLL's path: the proxy's own folder.
pub(crate) fn core_dll_path(proxy_path: &Path) -> PathBuf {
    proxy_path.with_file_name(CORE_DLL)
}

/// Why the loader thread could not start the core. Every variant reports status 11 (core load failed).
#[derive(Debug, thiserror::Error)]
pub(crate) enum CoreLoadError {
    #[error("loading the core DLL failed")]
    Load(#[from] windows::core::Error),
    #[error("the core DLL has no init export")]
    MissingInit,
    #[error("the core's init export returned failure code {0}")]
    InitFailed(u32),
}

/// Maps the code the core's init export returns.
pub(crate) const fn init_result(code: u32) -> Result<(), CoreLoadError> {
    if code == INIT_STARTED { Ok(()) } else { Err(CoreLoadError::InitFailed(code)) }
}

#[cfg(test)]
mod tests {
    use super::{Action, CoreLoadError, Exe, action, core_dll_path, exe, init_result};
    use ipc_layout::launch_token::{LaunchToken, Mode, TokenError};
    use std::path::{Path, PathBuf};

    fn valid_token() -> LaunchToken {
        LaunchToken::new(Mode::Vr, 1, 2)
    }

    #[test]
    fn exe_matches_the_two_game_executables_in_any_case() {
        assert_eq!(exe(Path::new(r"C:\Games\Baldurs Gate 3\bin\bg3_dx11.exe")), Exe::Dx11);
        assert_eq!(exe(Path::new(r"D:\bin\BG3_DX11.EXE")), Exe::Dx11);
        assert_eq!(exe(Path::new(r"\\?\C:\bin\bg3.exe")), Exe::Vulkan);
        assert_eq!(exe(Path::new(r"C:\bin\Bg3.Exe")), Exe::Vulkan);
    }

    #[test]
    fn exe_treats_every_other_name_as_other() {
        for path in [
            r"C:\bin\bg3_dx11.exe.bak",
            r"C:\bin\bg3_dx12.exe",
            r"C:\bin\LariLauncher.exe",
            r"C:\bin\xbg3.exe",
            "bg3_dx11",
            "",
            r"C:\",
        ] {
            assert_eq!(exe(Path::new(path)), Exe::Other, "{path}");
        }
    }

    #[test]
    fn a_valid_token_in_the_dx11_exe_loads_the_core_with_the_token_mode() {
        assert_eq!(action(Exe::Dx11, Ok(valid_token())), Action::LoadCore { mode: 2 });
    }

    #[test]
    fn an_unknown_mode_reaches_the_core_undecoded() {
        let mut bytes = valid_token().encode();
        bytes[8] = 7;
        let token = LaunchToken::decode(&bytes);
        assert_eq!(action(Exe::Dx11, token), Action::LoadCore { mode: 7 });
    }

    #[test]
    fn a_valid_token_in_the_vulkan_exe_reports_the_wrong_renderer() {
        assert_eq!(action(Exe::Vulkan, Ok(valid_token())), Action::ReportWrongRenderer);
    }

    #[test]
    fn a_valid_token_in_another_exe_starts_nothing() {
        assert_eq!(action(Exe::Other, Ok(valid_token())), Action::StartNothing);
    }

    #[test]
    fn an_invalid_token_starts_nothing_in_every_exe() {
        let wrong_size = Err(TokenError::Size { len: 0 });
        let wrong_magic = Err(TokenError::Magic { found: 0 });
        let wrong_version = Err(TokenError::LayoutVersion { found: 9 });
        for exe in [Exe::Dx11, Exe::Vulkan, Exe::Other] {
            assert_eq!(action(exe, wrong_size), Action::StartNothing, "{exe:?}");
            assert_eq!(action(exe, wrong_magic), Action::StartNothing, "{exe:?}");
            assert_eq!(action(exe, wrong_version), Action::StartNothing, "{exe:?}");
        }
    }

    #[test]
    fn the_core_dll_path_is_next_to_the_proxy() {
        let proxy = Path::new(r"C:\Games\Baldurs Gate 3\bin\version.dll");
        let expected = PathBuf::from(r"C:\Games\Baldurs Gate 3\bin\arcane_eye_core.dll");
        assert_eq!(core_dll_path(proxy), expected);
    }

    #[test]
    fn only_init_code_zero_is_success() {
        assert!(matches!(init_result(0), Ok(())));
        assert!(matches!(init_result(1), Err(CoreLoadError::InitFailed(1))));
        assert!(matches!(init_result(6), Err(CoreLoadError::InitFailed(6))));
    }
}
