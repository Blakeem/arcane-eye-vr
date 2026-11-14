//! The command line: `trial --name <name>` and the options that set the steps.

use std::ffi::{OsStr, OsString};
use std::num::NonZeroU32;
use std::ops::RangeInclusive;
use std::path::PathBuf;
use std::time::Duration;

use ipc_layout::launch_token::Mode;
use viewer_client::region::Region;

pub(crate) const USAGE: &str = "usage: trial --name <name> [--mode vr|desktop-stereo] \
     [--warmup-secs <n>] [--perf-lines <n>] [--captures <n>] [--burst-frames <n>] \
     [--burst-region <x>,<y>,<width>,<height>] [--burst-step <k>] [--out <folder>] \
     [--game-dir <folder>] [--settings-file <path>] [--dev-file <path>]";

/// The most frames one trial captures, since a frame file's number has 4 digits.
const MAX_CAPTURES: u32 = 9999;

/// The most frames one burst keeps.
const MAX_BURST_FRAMES: u32 = 2000;

/// The largest burst step, which keeps every 16th pixel.
const MAX_BURST_STEP: u32 = 16;

/// The text that opens every option, and that no option value may start with.
const OPTION_START: &str = "--";

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

/// Everything the command line sets apart from the name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Settings {
    pub(crate) mode: Mode,
    pub(crate) warmup: Duration,
    pub(crate) perf_lines: u32,
    pub(crate) captures: u32,
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
            warmup: Duration::from_secs(20),
            perf_lines: 3,
            captures: 1,
            burst: Burst::default(),
            out: PathBuf::from("trial-results"),
            game_dir: None,
            player_file: None,
            dev_file: None,
        }
    }
}

/// The burst step's settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Burst {
    /// The count of frames the burst keeps, where 0 runs no burst step.
    pub(crate) frames: u32,
    /// The region of the frame the burst keeps, or `None` for the entire frame.
    pub(crate) region: Option<Region>,
    /// The burst keeps every `step`-th pixel of the region in both directions.
    pub(crate) step: NonZeroU32,
}

impl Default for Burst {
    fn default() -> Self {
        Self { frames: 0, region: None, step: NonZeroU32::MIN }
    }
}

/// One trial's command line.
#[derive(Clone, Debug, PartialEq, Eq)]
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
    #[error("{option} takes a whole number from {min} to {max}, not \"{value}\"")]
    OutOfRange { option: &'static str, value: String, min: u64, max: u64 },
    #[error(
        "--burst-region takes <x>,<y>,<width>,<height> in whole pixels, with a width and a height \
         above 0, not \"{0}\""
    )]
    BadRegion(String),
    #[error("trial got {0}, which needs --burst-frames above 0")]
    BurstWithoutFrames(&'static str),
}

/// The options the command line takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Flag {
    Name,
    Mode,
    WarmupSecs,
    PerfLines,
    Captures,
    BurstFrames,
    BurstRegion,
    BurstStep,
    Out,
    GameDir,
    SettingsFile,
    DevFile,
}

impl Flag {
    const ALL: [Self; 12] = [
        Self::Name,
        Self::Mode,
        Self::WarmupSecs,
        Self::PerfLines,
        Self::Captures,
        Self::BurstFrames,
        Self::BurstRegion,
        Self::BurstStep,
        Self::Out,
        Self::GameDir,
        Self::SettingsFile,
        Self::DevFile,
    ];

    const fn as_str(self) -> &'static str {
        match self {
            Self::Name => "--name",
            Self::Mode => "--mode",
            Self::WarmupSecs => "--warmup-secs",
            Self::PerfLines => "--perf-lines",
            Self::Captures => "--captures",
            Self::BurstFrames => "--burst-frames",
            Self::BurstRegion => "--burst-region",
            Self::BurstStep => "--burst-step",
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
        let flag = Flag::named(&arg)?;
        if seen.contains(&flag) {
            return Err(ArgsError::Repeated(flag.as_str()));
        }
        seen.push(flag);
        let value = value_of(flag, args.next())?;
        match flag {
            Flag::Name => name = Some(TrialName::new(&text(&value))?),
            Flag::Mode => settings.mode = mode(&text(&value))?,
            Flag::WarmupSecs => settings.warmup = seconds(flag, &value)?,
            Flag::PerfLines => settings.perf_lines = count(flag, &value, 1..=u32::MAX)?,
            Flag::Captures => settings.captures = count(flag, &value, 1..=MAX_CAPTURES)?,
            Flag::BurstFrames => {
                settings.burst.frames = count(flag, &value, 0..=MAX_BURST_FRAMES)?;
            }
            Flag::BurstRegion => settings.burst.region = Some(region(&text(&value))?),
            Flag::BurstStep => settings.burst.step = step(flag, &value)?,
            Flag::Out => settings.out = PathBuf::from(value),
            Flag::GameDir => settings.game_dir = Some(PathBuf::from(value)),
            Flag::SettingsFile => settings.player_file = Some(PathBuf::from(value)),
            Flag::DevFile => settings.dev_file = Some(PathBuf::from(value)),
        }
    }
    let name = name.ok_or(ArgsError::MissingName)?;
    burst_has_frames(&seen, &settings.burst)?;
    Ok(Options { name, settings })
}

/// Fails when the command line sets the burst region or step and runs no burst, since the trial
/// would then end without the burst its person asked for.
fn burst_has_frames(seen: &[Flag], burst: &Burst) -> Result<(), ArgsError> {
    if burst.frames > 0 {
        return Ok(());
    }
    let needs_frames =
        [Flag::BurstRegion, Flag::BurstStep].into_iter().find(|flag| seen.contains(flag));
    match needs_frames {
        Some(flag) => Err(ArgsError::BurstWithoutFrames(flag.as_str())),
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

/// An argument as text, with a replacement character for each unit that is not Unicode.
fn text(value: &OsStr) -> String {
    value.to_string_lossy().into_owned()
}

fn mode(value: &str) -> Result<Mode, ArgsError> {
    let found = Mode::ALL.into_iter().find(|candidate| value == candidate.as_str());
    found.ok_or_else(|| ArgsError::UnknownMode(value.to_owned()))
}

/// A whole number of seconds.
fn seconds(flag: Flag, value: &OsStr) -> Result<Duration, ArgsError> {
    let value = text(value);
    match value.parse::<u64>() {
        Ok(seconds) => Ok(Duration::from_secs(seconds)),
        Err(_) => {
            Err(ArgsError::OutOfRange { option: flag.as_str(), value, min: 0, max: u64::MAX })
        }
    }
}

/// A count in `range`.
fn count(flag: Flag, value: &OsStr, range: RangeInclusive<u32>) -> Result<u32, ArgsError> {
    let value = text(value);
    match value.parse::<u32>() {
        Ok(count) if range.contains(&count) => Ok(count),
        Ok(_) | Err(_) => Err(ArgsError::OutOfRange {
            option: flag.as_str(),
            value,
            min: (*range.start()).into(),
            max: (*range.end()).into(),
        }),
    }
}

/// A burst step from 1 to [`MAX_BURST_STEP`].
fn step(flag: Flag, value: &OsStr) -> Result<NonZeroU32, ArgsError> {
    let value = text(value);
    match value.parse::<NonZeroU32>() {
        Ok(step) if step.get() <= MAX_BURST_STEP => Ok(step),
        Ok(_) | Err(_) => Err(ArgsError::OutOfRange {
            option: flag.as_str(),
            value,
            min: 1,
            max: MAX_BURST_STEP.into(),
        }),
    }
}

/// A region written `<x>,<y>,<width>,<height>` in whole pixels, with a width and a height above 0.
fn region(value: &str) -> Result<Region, ArgsError> {
    let numbers: Vec<Option<u32>> = value.split(',').map(|part| part.parse().ok()).collect();
    match numbers.as_slice() {
        &[Some(x), Some(y), Some(width), Some(height)] if width > 0 && height > 0 => {
            Ok(Region { x, y, width, height })
        }
        _ => Err(ArgsError::BadRegion(value.to_owned())),
    }
}

#[cfg(test)]
mod tests;
