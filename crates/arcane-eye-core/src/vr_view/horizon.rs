//! The horizon in use: the horizon a world frame's anchor is built from. The viewing side's horizon
//! wish wins while it holds one, and the dev flag `vr.horizon` holds in every other case.

use ipc_layout::viewer_header::horizon_wish::HorizonWish;
use settings::dev::Horizon;

/// Where the horizon in use comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HorizonFrom {
    /// The viewing side's horizon wish.
    Wish,
    /// The dev flag `vr.horizon`.
    Flag,
}

impl HorizonFrom {
    /// The name the `vr view` line gives it.
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Wish => "wish",
            Self::Flag => "flag",
        }
    }
}

/// The horizon in use and where it comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct HorizonInUse {
    pub(crate) horizon: Horizon,
    pub(crate) from: HorizonFrom,
}

impl HorizonInUse {
    /// The horizon in use while the viewing side's horizon wish is `wish`, `None` for no wish or a
    /// viewing side that does not beat, and the dev flag `vr.horizon` is `flag`.
    pub(crate) const fn of(wish: Option<HorizonWish>, flag: Horizon) -> Self {
        match wish {
            Some(HorizonWish::Level) => Self { horizon: Horizon::Level, from: HorizonFrom::Wish },
            Some(HorizonWish::Camera) => Self { horizon: Horizon::Camera, from: HorizonFrom::Wish },
            None => Self { horizon: flag, from: HorizonFrom::Flag },
        }
    }
}

#[cfg(test)]
mod tests {
    use ipc_layout::viewer_header::horizon_wish::HorizonWish;
    use settings::dev::Horizon;

    use super::{HorizonFrom, HorizonInUse};

    #[test]
    fn a_wish_wins_over_the_dev_flag_and_no_wish_leaves_the_dev_flag_in_use() {
        for flag in [Horizon::Level, Horizon::Camera] {
            let level = HorizonInUse::of(Some(HorizonWish::Level), flag);
            assert_eq!((level.horizon, level.from), (Horizon::Level, HorizonFrom::Wish));
            let camera = HorizonInUse::of(Some(HorizonWish::Camera), flag);
            assert_eq!((camera.horizon, camera.from), (Horizon::Camera, HorizonFrom::Wish));
            let none = HorizonInUse::of(None, flag);
            assert_eq!((none.horizon, none.from), (flag, HorizonFrom::Flag), "{flag:?}");
        }
        assert_eq!(
            [HorizonFrom::Wish, HorizonFrom::Flag].map(HorizonFrom::as_str),
            ["wish", "flag"]
        );
    }
}
