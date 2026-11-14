//! Guarded reads and writes of game memory and typed game addresses. Unsafe inside, safe API out.
//!
//! Every read and write copies through the shim's `__try` copy, so an unmapped or protected address
//! yields a failure value instead of a crash. A read copies plain data into a Rust value and a write
//! copies plain data out of one, so no reference into game memory exists.
//!
//! The safe API has one accepted gap. A [`GameAddr`] is a plain number, so a write cannot prove that
//! its address names memory no Rust value owns, and a write over a live Rust value is undefined
//! behavior that no type here prevents. The mod's callers write only to game memory and to pages the
//! mod commits for the game.

pub mod module;
mod raw;

/// An address in the game process. It is a plain number, so storing one keeps no borrow alive, and
/// reading through it goes through a guarded read again.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GameAddr(usize);

impl GameAddr {
    pub const fn new(address: usize) -> Self {
        Self(address)
    }

    pub const fn get(self) -> usize {
        self.0
    }

    /// The address `bytes` past this one, or `None` past the end of the address space.
    pub const fn checked_add(self, bytes: usize) -> Option<Self> {
        match self.0.checked_add(bytes) {
            Some(address) => Some(Self(address)),
            None => None,
        }
    }

    /// The address `bytes` past this one, as the destination of a write.
    ///
    /// # Errors
    ///
    /// Returns [`WriteError::PastAddressSpace`] past the end of the address space.
    pub const fn destination(self, bytes: usize) -> Result<Self, WriteError> {
        match self.checked_add(bytes) {
            Some(address) => Ok(address),
            None => Err(WriteError::PastAddressSpace),
        }
    }
}

mod sealed {
    pub trait Sealed {}
}

/// A type that every bit pattern of its size is a valid value of, so a guarded read can fill it with
/// any bytes. The trait is sealed, and only integers and floats implement it.
pub trait Plain: sealed::Sealed + Copy + Default {}

macro_rules! plain {
    ($($type:ty),+) => {
        $(
            impl sealed::Sealed for $type {}
            impl Plain for $type {}
        )+
    };
}

plain!(u8, u16, u32, u64, usize, i8, i16, i32, i64, isize, f32, f64);

/// The `T` stored at `address`, or `None` when any of its bytes cannot be read.
pub fn try_read<T: Plain>(address: GameAddr) -> Option<T> {
    raw::read(address)
}

/// The `N` values of `T` stored from `address` on, or `None` when any of their bytes cannot be read.
/// The copy is one guarded read and makes no heap allocation.
pub fn try_read_array<T: Plain, const N: usize>(address: GameAddr) -> Option<[T; N]> {
    let mut values = [T::default(); N];
    raw::read_into(address, &mut values)?;
    Some(values)
}

/// A copy of the `len` bytes at `address`, or `None` when any of them cannot be read or the buffer
/// for them cannot be allocated.
pub fn try_read_bytes(address: GameAddr, len: usize) -> Option<Vec<u8>> {
    let mut bytes = Vec::new();
    if bytes.try_reserve_exact(len).is_err() {
        return None;
    }
    bytes.resize(len, 0);
    raw::read_into(address, &mut bytes)?;
    Some(bytes)
}

/// Why a guarded write did not write all of its bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum WriteError {
    #[error("a guarded write to {address:#x} faulted, so it wrote none or part of its bytes")]
    Faulted { address: usize },
    /// A destination computed from a base address and an offset lies past the end of the address
    /// space, so no write was tried.
    #[error("the destination lies past the end of the address space")]
    PastAddressSpace,
}

/// Writes `value` to `address`.
///
/// The write trusts `address` to name game memory or memory the mod shares with the game, which no
/// Rust value owns. The guard catches a fault and nothing else, so a write over a live Rust value is
/// the undefined behavior the crate doc names.
///
/// # Errors
///
/// Returns [`WriteError::Faulted`] when any byte of the destination cannot be written. The
/// destination may then hold part of `value`.
pub fn try_write<T: Plain>(address: GameAddr, value: T) -> Result<(), WriteError> {
    try_write_slice(address, &[value])
}

/// Writes `values` to `address` and the bytes after it, in one guarded copy that makes no heap
/// allocation. The write trusts `address` as [`try_write`] does.
///
/// # Errors
///
/// Returns [`WriteError::Faulted`] when any byte of the destination cannot be written. The
/// destination may then hold part of `values`.
pub fn try_write_slice<T: Plain>(address: GameAddr, values: &[T]) -> Result<(), WriteError> {
    if raw::write_from(address, values) {
        Ok(())
    } else {
        Err(WriteError::Faulted { address: address.get() })
    }
}
