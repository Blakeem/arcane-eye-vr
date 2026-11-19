//! The controllers' action set: its actions, the bindings each device profile suggests, and the
//! spaces of the hands. The head's VIEW space is the session's.
//!
//! Each action exists for the two hands, so a profile's `[left]` and `[right]` tables bind the same
//! actions. A profile whose bindings the runtime refuses is left out.

use device_profile::profile::{ActionName, DeviceProfile, Hand};
use openxr::{
    Action, ActionInput, ActionSet, ActionState, ActionTy, Binding, D3D11, Haptic, Instance, Path,
    Posef, Space, Time, Vector2f,
};
use tracing::{info, warn};

use crate::error::{XrError, XrStep};
use crate::head::ViewSpace;
use crate::input::{self, Focus, HandInput, Input};
use crate::raw;

/// The action set's name and its name in the runtime's binding interface.
const SET_NAME: (&str, &str) = ("controls", "Controls");

/// The session's actions, each for both hands.
struct Actions {
    aim: Action<Posef>,
    grip: Action<Posef>,
    menu: Action<bool>,
    select: Action<f32>,
    secondary: Action<bool>,
    middle: Action<bool>,
    scroll: Action<Vector2f>,
    haptic: Action<Haptic>,
}

impl Actions {
    fn create(set: &ActionSet, hands: &[Path]) -> Result<Self, XrError> {
        Ok(Self {
            aim: create_action(set, ActionName::Aim, hands)?,
            grip: create_action(set, ActionName::Grip, hands)?,
            menu: create_action(set, ActionName::Menu, hands)?,
            select: create_action(set, ActionName::Select, hands)?,
            secondary: create_action(set, ActionName::Secondary, hands)?,
            middle: create_action(set, ActionName::Middle, hands)?,
            scroll: create_action(set, ActionName::Scroll, hands)?,
            haptic: create_action(set, ActionName::Haptic, hands)?,
        })
    }

    /// The binding of the action `name` to the input or output `path`.
    fn binding(&self, name: ActionName, path: Path) -> Binding<'_> {
        match name {
            ActionName::Aim => Binding::new(&self.aim, path),
            ActionName::Grip => Binding::new(&self.grip, path),
            ActionName::Menu => Binding::new(&self.menu, path),
            ActionName::Select => Binding::new(&self.select, path),
            ActionName::Secondary => Binding::new(&self.secondary, path),
            ActionName::Middle => Binding::new(&self.middle, path),
            ActionName::Scroll => Binding::new(&self.scroll, path),
            ActionName::Haptic => Binding::new(&self.haptic, path),
        }
    }
}

/// The action `name` of `set` for the subaction paths `hands`. Its key in a profile file is also its
/// name in the runtime's binding interface.
fn create_action<T: ActionTy>(
    set: &ActionSet,
    name: ActionName,
    hands: &[Path],
) -> Result<Action<T>, XrError> {
    let name = name.as_str();
    XrError::check(XrStep::CreateAction, set.create_action(name, name, hands))
}

/// One hand's subaction path and the spaces of its two pose actions.
struct HandSpaces {
    path: Path,
    aim: Space,
    grip: Space,
}

impl HandSpaces {
    fn create(
        session: &openxr::Session<D3D11>,
        actions: &Actions,
        path: Path,
    ) -> Result<Self, XrError> {
        let space = |action: &Action<Posef>| {
            let created = action.create_space(session, path, Posef::IDENTITY);
            XrError::check(XrStep::CreateActionSpace, created)
        };
        Ok(Self { path, aim: space(&actions.aim)?, grip: space(&actions.grip)? })
    }
}

/// What one input read needs: the session, the space the poses are in, the head's VIEW space and
/// the display time.
#[derive(Clone, Copy)]
pub(crate) struct ReadAt<'a> {
    pub(crate) session: &'a openxr::Session<D3D11>,
    pub(crate) base: &'a Space,
    pub(crate) head: &'a ViewSpace,
    pub(crate) time: Time,
}

/// The attached action set with its actions and the spaces of the hands.
pub(crate) struct Controls {
    left: HandSpaces,
    right: HandSpaces,
    actions: Actions,
    set: ActionSet,
}

impl Controls {
    /// The controls of `profiles` for `session`, or `None` when no profile is left. A failed step
    /// logs a warning and gives `None`, since the screen works without controllers.
    pub(crate) fn open(
        instance: &Instance,
        session: &openxr::Session<D3D11>,
        profiles: &[DeviceProfile],
    ) -> Option<Self> {
        if profiles.is_empty() {
            info!("no device profile is left, so the session reads no controller");
            return None;
        }
        match Self::create(instance, session, profiles) {
            Ok(controls) => controls,
            Err(error) => {
                warn!(error = %error, "the controllers' actions failed, so the session reads no controller");
                None
            }
        }
    }

    fn create(
        instance: &Instance,
        session: &openxr::Session<D3D11>,
        profiles: &[DeviceProfile],
    ) -> Result<Option<Self>, XrError> {
        let path = |hand: Hand| {
            XrError::check(XrStep::StringToPath, instance.string_to_path(hand.user_path()))
        };
        let hands = [path(Hand::Left)?, path(Hand::Right)?];
        let (name, localized) = SET_NAME;
        let created = instance.create_action_set(name, localized, 0);
        let set = XrError::check(XrStep::CreateActionSet, created)?;
        let actions = Actions::create(&set, &hands)?;
        let mut suggested = Vec::new();
        for profile in profiles {
            match suggest(instance, &actions, profile) {
                Ok(()) => suggested.push(profile.name.as_str()),
                Err(error) => warn!(
                    profile = %profile.name,
                    error = %error,
                    "the runtime refused the device profile's bindings, so the profile is left out"
                ),
            }
        }
        if suggested.is_empty() {
            warn!("the runtime refused every device profile, so the session reads no controller");
            return Ok(None);
        }
        XrError::check(XrStep::AttachActionSets, session.attach_action_sets(&[&set]))?;
        let [left, right] = hands;
        let left = HandSpaces::create(session, &actions, left)?;
        let right = HandSpaces::create(session, &actions, right)?;
        info!(profiles = ?suggested, "the session reads the controllers of these device profiles");
        Ok(Some(Self { left, right, actions, set }))
    }

    /// Syncs the actions and reads the input at `at`. A session without the input focus gives
    /// the idle input.
    pub(crate) fn read(&self, at: ReadAt<'_>) -> Result<Input, XrError> {
        if input::synced(raw::sync_actions(at.session, &self.set))? == Focus::Unfocused {
            return Ok(Input::default());
        }
        let head = at.head.locate(at.base, at.time)?;
        Ok(Input {
            left: self.hand(&self.left, at)?,
            right: self.hand(&self.right, at)?,
            head: input::tracked(head),
        })
    }

    fn hand(&self, spaces: &HandSpaces, at: ReadAt<'_>) -> Result<HandInput, XrError> {
        let (actions, path) = (&self.actions, spaces.path);
        Ok(HandInput {
            aim: pose(&actions.aim, &spaces.aim, path, at)?,
            grip: pose(&actions.grip, &spaces.grip, path, at)?,
            menu: input::button(state(&actions.menu, path, at)?),
            select: input::amount(state(&actions.select, path, at)?),
            secondary: input::button(state(&actions.secondary, path, at)?),
            middle: input::button(state(&actions.middle, path, at)?),
            scroll: input::stick(state(&actions.scroll, path, at)?),
        })
    }

    /// Logs each hand's current interaction profile, after the runtime reported a change.
    pub(crate) fn log_interaction_profiles(&self, session: &openxr::Session<D3D11>) {
        for (hand, spaces) in [(Hand::Left, &self.left), (Hand::Right, &self.right)] {
            match interaction_profile(session, spaces.path) {
                Ok(profile) => {
                    info!(hand = hand.as_str(), profile = %profile, "the hand's interaction profile");
                }
                Err(error) => warn!(
                    hand = hand.as_str(),
                    error = %error,
                    "the hand's interaction profile could not be read"
                ),
            }
        }
    }
}

/// Suggests the bindings of `profile` for its interaction profile.
fn suggest(instance: &Instance, actions: &Actions, profile: &DeviceProfile) -> Result<(), XrError> {
    let path = |text: &str| XrError::check(XrStep::StringToPath, instance.string_to_path(text));
    let interaction_profile = path(&profile.interaction_profile)?;
    let mut bindings = Vec::new();
    for hand in Hand::ALL {
        for name in ActionName::ALL {
            if let Some(bound) = profile.hand(hand).path(name) {
                bindings.push(actions.binding(name, path(bound)?));
            }
        }
    }
    let suggested = instance.suggest_interaction_profile_bindings(interaction_profile, &bindings);
    XrError::check(XrStep::SuggestBindings, suggested)
}

/// The state of `action` on the hand `path` at the last sync.
fn state<T: ActionInput>(
    action: &Action<T>,
    path: Path,
    at: ReadAt<'_>,
) -> Result<ActionState<T>, XrError> {
    XrError::check(XrStep::ReadAction, action.state(at.session, path))
}

/// The tracked pose of the pose action `action` on the hand `path`, located through its `space`.
fn pose(
    action: &Action<Posef>,
    space: &Space,
    path: Path,
    at: ReadAt<'_>,
) -> Result<Option<Posef>, XrError> {
    let active = XrError::check(XrStep::ReadAction, action.is_active(at.session, path))?;
    if !active {
        return Ok(None);
    }
    let location = XrError::check(XrStep::LocateSpace, space.locate(at.base, at.time))?;
    Ok(input::tracked(location))
}

/// The interaction profile path the runtime uses for the hand `path`, or `none` while it uses
/// none.
fn interaction_profile(session: &openxr::Session<D3D11>, path: Path) -> Result<String, XrError> {
    let current = session.current_interaction_profile(path);
    let profile = XrError::check(XrStep::ReadInteractionProfile, current)?;
    if profile == Path::NULL {
        return Ok("none".to_owned());
    }
    let text = session.instance().path_to_string(profile);
    XrError::check(XrStep::ReadInteractionProfile, text)
}
