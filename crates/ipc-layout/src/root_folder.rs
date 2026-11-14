//! The environment variable that gives a process the mod's root folder.
//!
//! The trial runner sets it on the game it starts, so the game's core reads its settings and writes its
//! session log in the trial's own folder. The core and the launch app read it. The launch app starts
//! the game through Steam, so the game gets Steam's environment and never the launch app's value.

/// The variable's name. Its value names the root folder when it is an absolute path.
pub const HOME_VARIABLE: &str = "ARCANE_EYE_VR_HOME";
