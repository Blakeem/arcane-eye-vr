//! The option values: text, the mode and scene names, numbers, the burst region, the limits and
//! the cursor position.

use std::ffi::OsStr;
use std::num::NonZeroU32;
use std::ops::RangeInclusive;
use std::time::Duration;

use ipc_layout::launch_token::Mode;
use launch::cursor::ClientPoint;
use viewer_client::region::Region;

use super::{ArgsError, Flag, MAX_BURST_STEP, Scene};
use crate::compare::Limit;

/// An argument as text, with a replacement character for each unit that is not Unicode.
pub(super) fn text(value: &OsStr) -> String {
    value.to_string_lossy().into_owned()
}

/// The mode whose name is `value`.
pub(super) fn mode(value: &str) -> Result<Mode, ArgsError> {
    let found = Mode::ALL.into_iter().find(|candidate| value == candidate.as_str());
    found.ok_or_else(|| ArgsError::UnknownMode(value.to_owned()))
}

/// The scene whose name is `value`.
pub(super) fn scene(value: &str) -> Result<Scene, ArgsError> {
    let found = Scene::ALL.into_iter().find(|candidate| value == candidate.as_str());
    found.ok_or_else(|| ArgsError::UnknownScene(value.to_owned()))
}

/// A whole number of seconds.
pub(super) fn seconds(flag: Flag, value: &OsStr) -> Result<Duration, ArgsError> {
    let value = text(value);
    match value.parse::<u64>() {
        Ok(seconds) => Ok(Duration::from_secs(seconds)),
        Err(_) => {
            Err(ArgsError::OutOfRange { option: flag.as_str(), value, min: 0, max: u64::MAX })
        }
    }
}

/// A count in `range`.
pub(super) fn count(
    flag: Flag,
    value: &OsStr,
    range: RangeInclusive<u32>,
) -> Result<u32, ArgsError> {
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
pub(super) fn step(flag: Flag, value: &OsStr) -> Result<NonZeroU32, ArgsError> {
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
pub(super) fn region(value: &str) -> Result<Region, ArgsError> {
    match numbers(value).as_slice() {
        &[Some(x), Some(y), Some(width), Some(height)] if width > 0 && height > 0 => {
            Ok(Region { x, y, width, height })
        }
        _ => Err(ArgsError::BadRegion(value.to_owned())),
    }
}

/// A largest difference: a finite decimal number from 0 up.
pub(super) fn limit(flag: Flag, value: &OsStr) -> Result<Limit, ArgsError> {
    let value = text(value);
    let parsed = value.parse::<f64>().ok().and_then(Limit::new);
    parsed.ok_or(ArgsError::BadLimit { option: flag.as_str(), value })
}

/// A client position written `<x>,<y>` in whole pixels.
pub(super) fn point(value: &str) -> Result<ClientPoint, ArgsError> {
    match numbers(value).as_slice() {
        &[Some(x), Some(y)] => Ok(ClientPoint { x, y }),
        _ => Err(ArgsError::BadCursor(value.to_owned())),
    }
}

/// The comma-separated parts of `value` as whole numbers, `None` for a part that is not one.
fn numbers(value: &str) -> Vec<Option<u32>> {
    value.split(',').map(|part| part.parse().ok()).collect()
}
