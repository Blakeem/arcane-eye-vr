//! The command line: `trial --name <name>` and the options that set the steps. `args\value.rs`
//! parses each option value that is not the trial name or a path.

use std::ffi::{OsStr, OsString};
use std::num::NonZeroU32;
use std::path::PathBuf;
use std::time::Duration;

use ipc_layout::launch_token::Mode;
use launch::cursor::ClientPoint;
use viewer_client::region::Region;

use self::value::{count, limit, mode, point, region, scene, seconds, step, text};
use crate::compare::Limit;

pub(crate) const USAGE: &str = "usage: trial --name <name> [--mode vr|desktop-stereo] \
     [--scene title|level] [--save <name>] [--background] [--warmup-secs <n>] [--perf-lines <n>] \
     [--captures <n>] [--max-half-difference <n>] [--cursor <x>,<y>] \
     [--burst-frames <n>] [--burst-region <x>,<y>,<width>,<height>] [--burst-step <k>] \
     [--max-burst-step <n>] \
     [--out <folder>] [--game-dir <folder>] [--settings-file <path>] [--dev-file <path>]";

/// The most frames one trial captures, since a frame file's number has 4 digits.
const MAX_CAPTURES: u32 = 9999;

/// The most frames one burst keeps.
const MAX_BURST_FRAMES: u32 = 2000;

/// The largest burst step, which keeps every 16th pixel.
const MAX_BURST_STEP: u32 = 16;

/// The text that opens every option, and that no option value may start with.
const OPTION_START: &str = "--";

/// The one option that takes no value.
const BACKGROUND: &str = "--background";

/// A trial name of ASCII letters, digits and hyphens, so it is safe in a folder name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TrialName(String);

impl TrialName {
    pub(crate) fn new(name: &str) -> Result<Self, ArgsError> {
        let allowed = |character: char| character.is_ascii_alphanumeric() || character == '-';
        if name.is_empty() || !name.chars().all(allowed) {
            return Err(ArgsError::BadName(name.to_owned()));
        }
        Ok(Self(name.to_owned()))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

/// Where in the game a trial runs its steps from the warm-up on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Scene {
    /// The title screen the game shows after its start.
    Title,
    /// The level of a save, the latest one unless the command line names one.
    Level,
}

impl Scene {
    const ALL: [Self; 2] = [Self::Title, Self::Level];

    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Title => "title",
            Self::Level => "level",
        }
    }
}

/// The Windows desktop the trial's game runs on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Background {
    /// The desktop the trial started on.
    Off,
    /// A desktop of the trial's own, so the game window takes neither the keyboard focus nor the
    /// mouse.
    On,
}

impl Background {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::On => "on",
        }
    }
}

/// Everything the command line sets apart from the name.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Settings {
    pub(crate) mode: Mode,
    pub(crate) scene: Scene,
    /// The name of the save the level scene loads, or `None` for the latest save.
    pub(crate) save: Option<String>,
    pub(crate) background: Background,
    pub(crate) warmup: Duration,
    /// The count of perf summary lines the perf window holds, where 0 runs no perf step.
    pub(crate) perf_lines: u32,
    /// The count of frames the capture step writes, where 0 writes none.
    pub(crate) captures: u32,
    /// The largest half difference a captured frame may have, or `None` for no limit.
    pub(crate) max_half_difference: Option<Limit>,
    /// The client position of the game window the cursor moves to before the captures.
    pub(crate) cursor: Option<ClientPoint>,
    pub(crate) burst: Burst,
    pub(crate) out: PathBuf,
    /// `None` finds the game's folder through Steam.
    pub(crate) game_dir: Option<PathBuf>,
    /// The file the game gets as its `settings.toml`, or `None` for no file.
    pub(crate) player_file: Option<PathBuf>,
    /// The file the game gets as its `dev.toml`, or `None` for no file.
    pub(crate) dev_file: Option<PathBuf>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            mode: Mode::DesktopStereo,
            scene: Scene::Title,
            save: None,
            background: Background::Off,
            warmup: Duration::from_secs(20),
            perf_lines: 3,
            captures: 1,
            max_half_difference: None,
            cursor: None,
            burst: Burst::default(),
            out: PathBuf::from("trial-results"),
            game_dir: None,
            player_file: None,
            dev_file: None,
        }
    }
}

/// The burst step's settings.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Burst {
    /// The count of frames the burst keeps, where 0 runs no burst step.
    pub(crate) frames: u32,
    /// The region of the frame the burst keeps, or `None` for the entire frame.
    pub(crate) region: Option<Region>,
    /// The burst keeps every `step`-th pixel of the region in both directions.
    pub(crate) step: NonZeroU32,
    /// The largest step a kept frame may take from the kept frame before it, or `None` for no
    /// limit.
    pub(crate) max_step: Option<Limit>,
}

impl Default for Burst {
    fn default() -> Self {
        Self { frames: 0, region: None, step: NonZeroU32::MIN, max_step: None }
    }
}

/// One trial's command line.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Options {
    pub(crate) name: TrialName,
    pub(crate) settings: Settings,
}

/// Why the command line names no trial.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub(crate) enum ArgsError {
    #[error("trial got the unknown argument \"{0}\"")]
    Unknown(String),
    #[error("trial got {0} with no value after it")]
    MissingValue(&'static str),
    #[error("trial got {0} more than once")]
    Repeated(&'static str),
    #[error("trial needs --name")]
    MissingName,
    #[error("the trial name \"{0}\" must be ASCII letters, digits and hyphens, and not empty")]
    BadName(String),
    #[error("trial got the unknown mode \"{0}\"")]
    UnknownMode(String),
    #[error("trial got the unknown scene \"{0}\"")]
    UnknownScene(String),
    #[error("{option} takes a whole number from {min} to {max}, not \"{value}\"")]
    OutOfRange { option: &'static str, value: String, min: u64, max: u64 },
    #[error(
        "--burst-region takes <x>,<y>,<width>,<height> in whole pixels, with a width and a height \
         above 0, not \"{0}\""
    )]
    BadRegion(String),
    #[error("{option} takes a decimal number from 0 up, such as 2.5, not \"{value}\"")]
    BadLimit { option: &'static str, value: String },
    #[error("--cursor takes <x>,<y> in whole pixels, not \"{0}\"")]
    BadCursor(String),
    #[error("trial got {0}, which needs --burst-frames above 0")]
    BurstWithoutFrames(&'static str),
    #[error("trial got {0}, which needs --captures above 0")]
    WithoutCaptures(&'static str),
    #[error("trial got --save, which needs --scene level")]
    SaveWithoutLevel,
}

/// The options that take a value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Flag {
    Name,
    Mode,
    Scene,
    Save,
    WarmupSecs,
    PerfLines,
    Captures,
    MaxHalfDifference,
    Cursor,
    BurstFrames,
    BurstRegion,
    BurstStep,
    MaxBurstStep,
    Out,
    GameDir,
    SettingsFile,
    DevFile,
}

impl Flag {
    const ALL: [Self; 17] = [
        Self::Name,
        Self::Mode,
        Self::Scene,
        Self::Save,
        Self::WarmupSecs,
        Self::PerfLines,
        Self::Captures,
        Self::MaxHalfDifference,
        Self::Cursor,
        Self::BurstFrames,
        Self::BurstRegion,
        Self::BurstStep,
        Self::MaxBurstStep,
        Self::Out,
        Self::GameDir,
        Self::SettingsFile,
        Self::DevFile,
    ];

    const fn as_str(self) -> &'static str {
        match self {
            Self::Name => "--name",
            Self::Mode => "--mode",
            Self::Scene => "--scene",
            Self::Save => "--save",
            Self::WarmupSecs => "--warmup-secs",
            Self::PerfLines => "--perf-lines",
            Self::Captures => "--captures",
            Self::MaxHalfDifference => "--max-half-difference",
            Self::Cursor => "--cursor",
            Self::BurstFrames => "--burst-frames",
            Self::BurstRegion => "--burst-region",
            Self::BurstStep => "--burst-step",
            Self::MaxBurstStep => "--max-burst-step",
            Self::Out => "--out",
            Self::GameDir => "--game-dir",
            Self::SettingsFile => "--settings-file",
            Self::DevFile => "--dev-file",
        }
    }

    fn named(arg: &OsStr) -> Result<Self, ArgsError> {
        let found = Self::ALL.into_iter().find(|flag| arg == flag.as_str());
        found.ok_or_else(|| ArgsError::Unknown(arg.to_string_lossy().into_owned()))
    }
}

/// The trial that `args`, the command line without the program name, asks for.
pub(crate) fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Options, ArgsError> {
    let mut name = None;
    let mut settings = Settings::default();
    let mut seen: Vec<Flag> = Vec::new();
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        if arg == BACKGROUND {
            if settings.background == Background::On {
                return Err(ArgsError::Repeated(BACKGROUND));
            }
            settings.background = Background::On;
            continue;
        }
        let flag = Flag::named(&arg)?;
        if seen.contains(&flag) {
            return Err(ArgsError::Repeated(flag.as_str()));
        }
        seen.push(flag);
        let value = value_of(flag, args.next())?;
        set(flag, &value, &mut name, &mut settings)?;
    }
    let name = name.ok_or(ArgsError::MissingName)?;
    needs_count(&seen, settings.burst.frames, &BURST_OPTIONS, ArgsError::BurstWithoutFrames)?;
    needs_count(&seen, settings.captures, &CAPTURE_OPTIONS, ArgsError::WithoutCaptures)?;
    if settings.save.is_some() && settings.scene != Scene::Level {
        return Err(ArgsError::SaveWithoutLevel);
    }
    Ok(Options { name, settings })
}

/// Sets the name or the setting that `flag` names to `value`.
fn set(
    flag: Flag,
    value: &OsStr,
    name: &mut Option<TrialName>,
    settings: &mut Settings,
) -> Result<(), ArgsError> {
    match flag {
        Flag::Name => *name = Some(TrialName::new(&text(value))?),
        Flag::Mode => settings.mode = mode(&text(value))?,
        Flag::Scene => settings.scene = scene(&text(value))?,
        Flag::Save => settings.save = Some(text(value)),
        Flag::WarmupSecs => settings.warmup = seconds(flag, value)?,
        Flag::PerfLines => settings.perf_lines = count(flag, value, 0..=u32::MAX)?,
        Flag::Captures => settings.captures = count(flag, value, 0..=MAX_CAPTURES)?,
        Flag::MaxHalfDifference => settings.max_half_difference = Some(limit(flag, value)?),
        Flag::Cursor => settings.cursor = Some(point(&text(value))?),
        Flag::BurstFrames => settings.burst.frames = count(flag, value, 0..=MAX_BURST_FRAMES)?,
        Flag::BurstRegion => settings.burst.region = Some(region(&text(value))?),
        Flag::BurstStep => settings.burst.step = step(flag, value)?,
        Flag::MaxBurstStep => settings.burst.max_step = Some(limit(flag, value)?),
        Flag::Out => settings.out = PathBuf::from(value),
        Flag::GameDir => settings.game_dir = Some(PathBuf::from(value)),
        Flag::SettingsFile => settings.player_file = Some(PathBuf::from(value)),
        Flag::DevFile => settings.dev_file = Some(PathBuf::from(value)),
    }
    Ok(())
}

/// The options that set the burst, which runs only with `--burst-frames` above 0.
const BURST_OPTIONS: [Flag; 3] = [Flag::BurstRegion, Flag::BurstStep, Flag::MaxBurstStep];

/// The options that act on the captures, which run only with `--captures` above 0.
const CAPTURE_OPTIONS: [Flag; 2] = [Flag::MaxHalfDifference, Flag::Cursor];

/// Fails with `error` when the command line sets one of `options` and `count` is 0, since the trial
/// would then end without what its person asked for.
fn needs_count(
    seen: &[Flag],
    count: u32,
    options: &[Flag],
    error: fn(&'static str) -> ArgsError,
) -> Result<(), ArgsError> {
    if count > 0 {
        return Ok(());
    }
    match options.iter().find(|flag| seen.contains(flag)) {
        Some(flag) => Err(error(flag.as_str())),
        None => Ok(()),
    }
}

/// The value after `flag`. A next argument that opens like an option is a missing value.
fn value_of(flag: Flag, next: Option<OsString>) -> Result<OsString, ArgsError> {
    match next {
        Some(value) if !text(&value).starts_with(OPTION_START) => Ok(value),
        Some(_) | None => Err(ArgsError::MissingValue(flag.as_str())),
    }
}

mod value;

#[cfg(test)]
mod tests;
