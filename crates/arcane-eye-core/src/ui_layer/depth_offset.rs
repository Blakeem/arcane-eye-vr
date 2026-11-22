//! The depth offset in use: the share of a half's width by which the UI layer moves the right
//! half's scene to the right, and the UI limit that goes with it, the largest UI disparity the UI
//! depth gives.
//!
//! While the viewing side reports the parallel share of its headset screen, the depth offset in use
//! is that share, at most [`SCREEN_MAX`], so the far scene sits where the eyes look parallel, or
//! nearer on a screen whose share is above [`SCREEN_MAX`]. The UI limit is that offset, so the UI
//! comes no nearer than half the screen's distance.
//! Otherwise the player setting `stereo.depth_offset_percent` applies with the UI limit
//! [`UI_LIMIT`].

use ipc_layout::viewer_header::screen::ParallelShare;
use settings::player::DepthOffsetPercent;

/// The largest UI disparity with the player setting. Past it the eye distance shrinks.
const UI_LIMIT: f32 = 0.04;

/// The largest depth offset a screen gives, which keeps the right half's black columns to 3 percent
/// of its width, rounded to entire columns, on a screen narrower than the eye distance divided by
/// this share.
const SCREEN_MAX: f32 = 0.03;

// A screen's UI limit is its depth offset, which is the smaller of UI_LIMIT and that offset only
// while SCREEN_MAX is at most UI_LIMIT.
const _: () = assert!(SCREEN_MAX <= UI_LIMIT);

/// The divisor that turns a percentage into a share.
const PERCENT: f32 = 100.0;

/// Where the depth offset in use comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OffsetFrom {
    /// The parallel share the viewing side reports for its headset screen.
    Screen,
    /// The player setting `stereo.depth_offset_percent`.
    Setting,
    /// No depth offset, in the launch mode `vr` or before the UI layer's first frame.
    NoOffset,
}

impl OffsetFrom {
    /// The name the `ui depth` line gives it.
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Screen => "screen",
            Self::Setting => "setting",
            Self::NoOffset => "none",
        }
    }
}

/// The depth offset in use as a share, and where it comes from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct DepthOffset {
    share: f32,
    origin: OffsetFrom,
}

impl DepthOffset {
    /// No depth offset, as in the launch mode `vr`, with the UI limit of the player setting.
    pub(crate) const NONE: Self = Self { share: 0.0, origin: OffsetFrom::NoOffset };

    /// The depth offset in use: the parallel share `screen` at most [`SCREEN_MAX`] while the
    /// viewing side reports one, and the player setting `setting` otherwise.
    pub(crate) fn in_use(setting: DepthOffsetPercent, screen: Option<ParallelShare>) -> Self {
        match screen {
            Some(screen) => {
                Self { share: screen.share().min(SCREEN_MAX), origin: OffsetFrom::Screen }
            }
            None => Self::of_setting(setting),
        }
    }

    /// The depth offset of the player setting `setting`.
    pub(crate) fn of_setting(setting: DepthOffsetPercent) -> Self {
        Self { share: setting.percent() / PERCENT, origin: OffsetFrom::Setting }
    }

    pub(crate) const fn share(self) -> f32 {
        self.share
    }

    pub(crate) const fn origin(self) -> OffsetFrom {
        self.origin
    }

    /// The UI limit in use: the depth offset from a screen, and [`UI_LIMIT`] otherwise.
    pub(crate) const fn ui_limit(self) -> f32 {
        match self.origin {
            OffsetFrom::Screen => self.share,
            OffsetFrom::Setting | OffsetFrom::NoOffset => UI_LIMIT,
        }
    }
}

#[cfg(test)]
mod tests {
    use ipc_layout::viewer_header::screen::ParallelShare;
    use settings::player::DepthOffsetPercent;

    use super::{DepthOffset, OffsetFrom};

    fn setting(percent: f32) -> DepthOffsetPercent {
        DepthOffsetPercent::try_from(percent).expect("a depth offset in range")
    }

    fn screen(share: f32) -> DepthOffset {
        let share = ParallelShare::new(share).expect("a share above 0");
        DepthOffset::in_use(setting(1.5), Some(share))
    }

    fn assert_near(value: f32, expected: f32, what: &str) {
        assert!((value - expected).abs() < 1e-7, "{what}: {value} is not {expected}");
    }

    #[test]
    fn a_screen_s_share_is_the_depth_offset_in_use_up_to_0_03() {
        assert_near(screen(0.021_666_7).share(), 0.021_666_7, "the 3 m screen");
        assert_near(screen(0.065).share(), 0.03, "a 1 m screen");
        assert_near(screen(0.004).share(), 0.004, "a screen of 16 m");
        assert_eq!(screen(0.004).origin(), OffsetFrom::Screen);
    }

    #[test]
    fn the_ui_limit_in_use_is_the_screen_s_depth_offset_and_0_04_with_the_setting() {
        assert_near(screen(0.021_666_7).ui_limit(), 0.021_666_7, "the 3 m screen");
        assert_near(screen(0.065).ui_limit(), 0.03, "a 1 m screen");
        assert_near(DepthOffset::in_use(setting(5.0), None).ui_limit(), 0.04, "the setting");
    }

    #[test]
    fn with_no_screen_the_player_setting_is_the_depth_offset_in_use() {
        let in_use = DepthOffset::in_use(setting(1.5), None);
        assert_near(in_use.share(), 0.015, "1.5 percent");
        assert_eq!(in_use.origin(), OffsetFrom::Setting);
    }

    #[test]
    fn no_depth_offset_comes_from_none_with_the_ui_limit_0_04() {
        assert_eq!(DepthOffset::NONE.share(), 0.0);
        assert_eq!(DepthOffset::NONE.origin(), OffsetFrom::NoOffset);
        assert_near(DepthOffset::NONE.ui_limit(), 0.04, "no offset");
    }

    #[test]
    fn the_ui_depth_line_names_where_the_depth_offset_comes_from() {
        assert_eq!(
            [OffsetFrom::Screen, OffsetFrom::Setting, OffsetFrom::NoOffset].map(OffsetFrom::as_str),
            ["screen", "setting", "none"]
        );
    }
}
