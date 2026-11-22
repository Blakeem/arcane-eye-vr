//! One device profile file: the [`DeviceProfile`] it holds and its parser.
//!
//! The file names an OpenXR interaction profile, the extension that enables it, if any, and for each
//! hand the input or output path of each action. An unknown key rejects the file.

use serde::Deserialize;

/// The only valid value of the top-level key `version`.
const FORMAT_VERSION: i64 = 1;

/// The start of every OpenXR interaction profile path.
const INTERACTION_PROFILE_PREFIX: &str = "/interaction_profiles/";

/// A hand that holds a controller.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hand {
    Left,
    Right,
}

impl Hand {
    pub const ALL: [Self; 2] = [Self::Left, Self::Right];

    /// The hand that is not this one.
    #[must_use]
    pub const fn other(self) -> Self {
        match self {
            Self::Left => Self::Right,
            Self::Right => Self::Left,
        }
    }

    /// The hand's name, which is also its table's name in a profile file.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Right => "right",
        }
    }

    /// The hand's OpenXR top level user path.
    pub const fn user_path(self) -> &'static str {
        match self {
            Self::Left => "/user/hand/left",
            Self::Right => "/user/hand/right",
        }
    }

    /// The start of every path the hand's table binds.
    pub const fn path_prefix(self) -> &'static str {
        match self {
            Self::Left => "/user/hand/left/",
            Self::Right => "/user/hand/right/",
        }
    }
}

/// An action of the session, which is also its key in a hand's table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionName {
    Aim,
    Grip,
    Menu,
    Select,
    Secondary,
    Scroll,
    Haptic,
}

impl ActionName {
    pub const ALL: [Self; 7] = [
        Self::Aim,
        Self::Grip,
        Self::Menu,
        Self::Select,
        Self::Secondary,
        Self::Scroll,
        Self::Haptic,
    ];

    /// The action's key in a hand's table.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Aim => "aim",
            Self::Grip => "grip",
            Self::Menu => "menu",
            Self::Select => "select",
            Self::Secondary => "secondary",
            Self::Scroll => "scroll",
            Self::Haptic => "haptic",
        }
    }
}

/// One controller's bindings of the session's actions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceProfile {
    /// The controller's name, for the logs.
    pub name: String,
    /// The OpenXR interaction profile path, such as `/interaction_profiles/khr/simple_controller`.
    pub interaction_profile: String,
    /// The OpenXR extension the interaction profile needs, if any.
    pub extension: Option<String>,
    pub left: HandBindings,
    pub right: HandBindings,
}

impl DeviceProfile {
    /// The bindings of `hand`.
    pub const fn hand(&self, hand: Hand) -> &HandBindings {
        match hand {
            Hand::Left => &self.left,
            Hand::Right => &self.right,
        }
    }
}

/// The path each action of one hand binds to. A missing key leaves its action unbound on the
/// controller.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HandBindings {
    pub aim: Option<String>,
    pub grip: Option<String>,
    pub menu: Option<String>,
    pub select: Option<String>,
    pub secondary: Option<String>,
    pub scroll: Option<String>,
    pub haptic: Option<String>,
}

impl HandBindings {
    /// The path `action` binds to, if any.
    pub fn path(&self, action: ActionName) -> Option<&str> {
        let path = match action {
            ActionName::Aim => &self.aim,
            ActionName::Grip => &self.grip,
            ActionName::Menu => &self.menu,
            ActionName::Select => &self.select,
            ActionName::Secondary => &self.secondary,
            ActionName::Scroll => &self.scroll,
            ActionName::Haptic => &self.haptic,
        };
        path.as_deref()
    }
}

/// The file as TOML gives it, before the checks of [`parse`].
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProfileFile {
    version: i64,
    name: String,
    interaction_profile: String,
    extension: Option<String>,
    #[serde(default)]
    left: HandBindings,
    #[serde(default)]
    right: HandBindings,
}

/// Why a profile file is rejected.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Rejection {
    #[error("{reason} at line {line}")]
    Toml { reason: String, line: usize },
    #[error("{reason}")]
    TomlUnplaced { reason: String },
    #[error("version must be {valid}, got {0}", valid = FORMAT_VERSION)]
    Version(i64),
    #[error("interaction_profile must start with {INTERACTION_PROFILE_PREFIX}, got {0}")]
    InteractionProfile(String),
    #[error(
        "{}.{} must start with {}, got {path}",
        hand.as_str(),
        action.as_str(),
        hand.path_prefix()
    )]
    HandPath { hand: Hand, action: ActionName, path: String },
    #[error("[left] and [right] bind no action")]
    NoBinding,
}

/// Parses the text of one profile file.
///
/// # Errors
///
/// Returns the [`Rejection`] of a text that is not valid TOML, lacks a required key, holds an unknown
/// key, a `version` other than 1, an `interaction_profile` that does not start with
/// `/interaction_profiles/`, a binding path outside its hand's table, or no binding.
pub fn parse(text: &str) -> Result<DeviceProfile, Rejection> {
    let file: ProfileFile = match toml::from_str(text) {
        Ok(file) => file,
        Err(error) => return Err(toml_rejection(text, &error)),
    };
    if file.version != FORMAT_VERSION {
        return Err(Rejection::Version(file.version));
    }
    if !file.interaction_profile.starts_with(INTERACTION_PROFILE_PREFIX) {
        return Err(Rejection::InteractionProfile(file.interaction_profile));
    }
    let profile = DeviceProfile {
        name: file.name,
        interaction_profile: file.interaction_profile,
        extension: file.extension,
        left: file.left,
        right: file.right,
    };
    check_hands(&profile)?;
    Ok(profile)
}

/// Rejects a path of a hand's table that lies outside that hand, and a profile that binds no action,
/// since OpenXR requires at least one binding in each suggestion.
fn check_hands(profile: &DeviceProfile) -> Result<(), Rejection> {
    if profile.left == HandBindings::default() && profile.right == HandBindings::default() {
        return Err(Rejection::NoBinding);
    }
    for hand in Hand::ALL {
        for action in ActionName::ALL {
            let Some(path) = profile.hand(hand).path(action) else {
                continue;
            };
            if !path.starts_with(hand.path_prefix()) {
                return Err(Rejection::HandPath { hand, action, path: path.to_owned() });
            }
        }
    }
    Ok(())
}

/// The rejection of `text` that the TOML `error` describes, on the line where the parser places it.
fn toml_rejection(text: &str, error: &toml::de::Error) -> Rejection {
    let reason = error.message().to_owned();
    let line = error.span().and_then(|span| text.get(..span.start)).map(line_of_end);
    match line {
        Some(line) => Rejection::Toml { reason, line },
        None => Rejection::TomlUnplaced { reason },
    }
}

/// The line, counted from 1, on which `before` ends.
fn line_of_end(before: &str) -> usize {
    before.matches('\n').count().saturating_add(1)
}

#[cfg(test)]
mod tests;
