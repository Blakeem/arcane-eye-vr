//! The named event that asks a running core to read the settings files again.
//!
//! The core creates the event at start, and its settings thread waits on it. The launch app's
//! `--reload-settings` opens the event by name and signals it. The event exists only while a core's
//! settings thread runs. So with no event, either no game with the mod runs in the session, or the
//! core's settings thread did not start or has ended.

/// The event's name, in the session namespace as the launch token's mapping name is.
pub const EVENT_NAME: &str = r"Local\arcane-eye-vr-reload";
