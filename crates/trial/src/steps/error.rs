//! Why a step failed: one variant per failure of the steps and of the parts they drive.

use std::io;
use std::path::PathBuf;
use std::process::ExitStatus;

use launch::checks::{GameRunning, RuntimeError};
use launch::cursor::CursorError;
use launch::direct::StartError;
use launch::game_dir::GameDirError;
use launch::status::WaitError;
use launch::stop::StopError;
use launch::token::TokenCreateError;
use launch::window_copy::WindowCopyError;
use viewer_client::frame::ReadError;

use super::Step;
use crate::background::Refused;
use crate::burst::BurstError;
use crate::compare::CompareError;
use crate::folder::FolderError;
use crate::headset::HeadsetError;
use crate::home::HomeError;
use crate::level::WaitFailed;
use crate::log_tail::TailError;
use crate::mod_check::ModCheckError;
use crate::perf::PerfError;
use crate::pixels::SizeError;
use crate::place::PlaceError;
use crate::png_file::PngError;

/// Why a step failed.
#[derive(Debug, thiserror::Error)]
pub(crate) enum StepError {
    #[error(transparent)]
    GameRunning(#[from] GameRunning),
    #[error(transparent)]
    Refused(#[from] Refused),
    #[error(transparent)]
    GameDir(#[from] GameDirError),
    #[error("the game's bin folder {} lacks {}", bin.display(), files.join(" and "))]
    ModMissing { bin: PathBuf, files: Vec<&'static str> },
    #[error("could not check the mod file {}", path.display())]
    ModCheck { path: PathBuf, source: io::Error },
    #[error(transparent)]
    Token(#[from] TokenCreateError),
    #[error(transparent)]
    Start(#[from] StartError),
    #[error(transparent)]
    Wait(#[from] WaitError),
    #[error(transparent)]
    Runtime(#[from] RuntimeError),
    #[error("the game exited during the {} step ({status})", step.as_str())]
    GameExited {
        step: Step,
        status: ExitStatus,
        /// The step's own failure, which the exit may explain, such as a timeout.
        #[source]
        failure: Option<Box<Self>>,
    },
    #[error("could not check whether the game runs")]
    GameState(#[source] io::Error),
    #[error(transparent)]
    Home(#[from] HomeError),
    #[error("the core wrote no session log in {} after the game started", log_dir.display())]
    NoCoreLog { log_dir: PathBuf },
    #[error(transparent)]
    Tail(#[from] TailError),
    #[error(transparent)]
    Perf(#[from] PerfError),
    #[error(transparent)]
    Place(#[from] PlaceError),
    #[error(transparent)]
    LevelWait(#[from] WaitFailed),
    #[error(transparent)]
    ModCheckFolder(#[from] ModCheckError),
    #[error(transparent)]
    Folder(#[from] FolderError),
    #[error("could not start the heartbeat thread")]
    Heartbeat(#[source] io::Error),
    #[error(transparent)]
    Read(#[from] ReadError),
    #[error(transparent)]
    Size(#[from] SizeError),
    #[error(transparent)]
    Png(#[from] PngError),
    #[error(transparent)]
    Cursor(#[from] CursorError),
    #[error(transparent)]
    Compare(#[from] CompareError),
    #[error(transparent)]
    Burst(#[from] BurstError),
    #[error(transparent)]
    Headset(#[from] HeadsetError),
    #[error(transparent)]
    WindowCopy(#[from] WindowCopyError),
    #[error(transparent)]
    Stop(#[from] StopError),
    #[error("could not copy the session log {} to {}", from.display(), to.display())]
    Copy { from: PathBuf, to: PathBuf, source: io::Error },
    #[error("the {} step did not pass first", .0.as_str())]
    Needs(Step),
}
