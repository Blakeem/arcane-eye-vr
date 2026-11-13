//! The launch token: the 64-byte block the launch app shares with the game process.
//!
//! The launch app creates the mapping [`MAPPING_NAME`] and fills every field. The proxy DLL and the core
//! write back only the status and the game process id, at [`STATUS_OFFSET`] and [`GAME_PID_OFFSET`].
//! Fields are little-endian bytes, so encode and decode never cast shared memory to a Rust type.

use core::mem::offset_of;

/// The name of the launch token's file mapping.
pub const MAPPING_NAME: &str = r"Local\arcane-eye-vr-launch";

/// The first field of every launch token, the ASCII bytes `B3SL`.
pub const MAGIC: u32 = u32::from_le_bytes(*b"B3SL");

/// The layout version this crate reads and writes. Any change to the layout increments it.
pub const LAYOUT_VERSION: u32 = 1;

/// The size of the token, and of its mapping, in bytes.
pub const SIZE: usize = 64;

/// The byte offset of the `u32` status field, which the proxy DLL and the core write.
pub const STATUS_OFFSET: usize = offset_of!(LaunchToken, status);

/// The byte offset of the `u32` game process id, which the proxy DLL writes.
pub const GAME_PID_OFFSET: usize = offset_of!(LaunchToken, game_pid);

const RESERVED_SIZE: usize = 32;

/// The launch token's fields, in layout order.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LaunchToken {
    magic: u32,
    layout_version: u32,
    mode: u32,
    launch_app_pid: u32,
    created_unix_ms: u64,
    status: u32,
    game_pid: u32,
    reserved: [u8; RESERVED_SIZE],
}

const _: () = assert!(size_of::<LaunchToken>() == SIZE);
const _: () = assert!(offset_of!(LaunchToken, magic) == 0);
const _: () = assert!(offset_of!(LaunchToken, layout_version) == 4);
const _: () = assert!(offset_of!(LaunchToken, mode) == 8);
const _: () = assert!(offset_of!(LaunchToken, launch_app_pid) == 12);
const _: () = assert!(offset_of!(LaunchToken, created_unix_ms) == 16);
const _: () = assert!(offset_of!(LaunchToken, status) == 24);
const _: () = assert!(offset_of!(LaunchToken, game_pid) == 28);
const _: () = assert!(offset_of!(LaunchToken, reserved) == 32);

impl LaunchToken {
    /// A token as the launch app creates it: status waiting and no game process id yet.
    pub const fn new(mode: Mode, launch_app_pid: u32, created_unix_ms: u64) -> Self {
        Self {
            magic: MAGIC,
            layout_version: LAYOUT_VERSION,
            mode: mode as u32,
            launch_app_pid,
            created_unix_ms,
            status: Status::Waiting as u32,
            game_pid: 0,
            reserved: [0; RESERVED_SIZE],
        }
    }

    /// The token's bytes in layout order.
    pub fn encode(&self) -> [u8; SIZE] {
        let field_bytes = self
            .magic
            .to_le_bytes()
            .into_iter()
            .chain(self.layout_version.to_le_bytes())
            .chain(self.mode.to_le_bytes())
            .chain(self.launch_app_pid.to_le_bytes())
            .chain(self.created_unix_ms.to_le_bytes())
            .chain(self.status.to_le_bytes())
            .chain(self.game_pid.to_le_bytes())
            .chain(self.reserved);
        let mut bytes = [0; SIZE];
        for (byte, field_byte) in bytes.iter_mut().zip(field_bytes) {
            *byte = field_byte;
        }
        bytes
    }

    /// Reads a token from its bytes and checks its magic value and layout version.
    ///
    /// The mode and status stay raw values here. [`LaunchToken::mode`] and [`LaunchToken::status`]
    /// decode them, so a reader that needs only one field is not rejected for the other.
    ///
    /// # Errors
    ///
    /// Returns [`TokenError`] when `bytes` is not 64 bytes long or holds a different magic value or
    /// layout version.
    pub fn decode(bytes: &[u8]) -> Result<Self, TokenError> {
        let token = Self::read_fields(bytes).ok_or(TokenError::Size { len: bytes.len() })?;
        if token.magic != MAGIC {
            return Err(TokenError::Magic { found: token.magic });
        }
        if token.layout_version != LAYOUT_VERSION {
            return Err(TokenError::LayoutVersion { found: token.layout_version });
        }
        Ok(token)
    }

    /// The mode field as stored, for a reader that passes it on undecoded.
    pub const fn raw_mode(&self) -> u32 {
        self.mode
    }

    /// The mode the launch app asked for.
    ///
    /// # Errors
    ///
    /// Returns [`UnknownMode`] when the mode field holds no [`Mode`] value.
    pub fn mode(&self) -> Result<Mode, UnknownMode> {
        Mode::try_from(self.mode)
    }

    /// The progress the game process last reported.
    ///
    /// # Errors
    ///
    /// Returns [`UnknownStatus`] when the status field holds no [`Status`] value.
    pub fn status(&self) -> Result<Status, UnknownStatus> {
        Status::try_from(self.status)
    }

    /// The game process id, or 0 before the proxy DLL wrote it.
    pub const fn game_pid(&self) -> u32 {
        self.game_pid
    }

    fn read_fields(mut bytes: &[u8]) -> Option<Self> {
        let magic = u32::from_le_bytes(take(&mut bytes)?);
        let layout_version = u32::from_le_bytes(take(&mut bytes)?);
        let mode = u32::from_le_bytes(take(&mut bytes)?);
        let launch_app_pid = u32::from_le_bytes(take(&mut bytes)?);
        let created_unix_ms = u64::from_le_bytes(take(&mut bytes)?);
        let status = u32::from_le_bytes(take(&mut bytes)?);
        let game_pid = u32::from_le_bytes(take(&mut bytes)?);
        let reserved = take(&mut bytes)?;
        let token = Self {
            magic,
            layout_version,
            mode,
            launch_app_pid,
            created_unix_ms,
            status,
            game_pid,
            reserved,
        };
        bytes.is_empty().then_some(token)
    }
}

/// Splits the next `N` bytes off the front of `bytes`.
fn take<const N: usize>(bytes: &mut &[u8]) -> Option<[u8; N]> {
    let (head, tail) = bytes.split_first_chunk::<N>()?;
    *bytes = tail;
    Some(*head)
}

/// The output mode the launch app asks for. Off has no value, since off means no token exists.
#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    DesktopStereo = 1,
    Vr = 2,
}

impl Mode {
    /// Every mode, for a reader that matches a mode by its value or its name.
    pub const ALL: [Self; 2] = [Self::DesktopStereo, Self::Vr];

    /// The mode's name, as the launch app's `--mode` argument spells it.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::DesktopStereo => "desktop-stereo",
            Self::Vr => "vr",
        }
    }
}

impl TryFrom<u32> for Mode {
    type Error = UnknownMode;

    fn try_from(value: u32) -> Result<Self, UnknownMode> {
        Self::ALL.into_iter().find(|mode| *mode as u32 == value).ok_or(UnknownMode(value))
    }
}

/// The progress the game process reports to the launch app.
#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Waiting = 0,
    CoreLoading = 1,
    HooksInstalled = 2,
    FirstPresent = 3,
    VrSessionRunning = 4,
    WrongRenderer = 10,
    CoreLoadFailed = 11,
    SignatureFailure = 12,
}

impl Status {
    const ALL: [Self; 8] = [
        Self::Waiting,
        Self::CoreLoading,
        Self::HooksInstalled,
        Self::FirstPresent,
        Self::VrSessionRunning,
        Self::WrongRenderer,
        Self::CoreLoadFailed,
        Self::SignatureFailure,
    ];
}

impl TryFrom<u32> for Status {
    type Error = UnknownStatus;

    fn try_from(value: u32) -> Result<Self, UnknownStatus> {
        Self::ALL.into_iter().find(|status| *status as u32 == value).ok_or(UnknownStatus(value))
    }
}

/// Why bytes are not a launch token this crate can read.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum TokenError {
    #[error("the launch token is {len} bytes, not {SIZE}")]
    Size { len: usize },
    #[error("the launch token's magic value is {found:#010x}, not {MAGIC:#010x}")]
    Magic { found: u32 },
    #[error("the launch token's layout version is {found}, not {LAYOUT_VERSION}")]
    LayoutVersion { found: u32 },
}

/// A mode field value that names no [`Mode`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("the launch mode value {0} names no mode")]
pub struct UnknownMode(pub u32);

/// A status field value that names no [`Status`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("the launch status value {0} names no status")]
pub struct UnknownStatus(pub u32);
