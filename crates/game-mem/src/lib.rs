//! Guarded reads and writes of game memory and typed game addresses. Unsafe inside, safe API out.
//!
//! Every read and write copies through the shim's `__try` copy, so an unmapped or protected address
//! yields a failure value instead of a crash. A read copies plain data into a Rust value and a write
//! copies plain data out of one, so no reference into game memory exists.
//!
//! A write goes only to a [`WriteDest`], a byte range that only the `unsafe fn` [`WriteDest::new`]
//! constructs, so safe code cannot write over a live Rust value.

use core::marker::PhantomData;

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
}

/// A byte range that guarded writes may fill for `'memory`, such as game memory or memory its owner
/// hands out for writes, which no Rust reference covers. It holds an address and a length and no
/// reference into the memory. Its methods sit with its unsafe constructor in `raw.rs`, since the lints
/// allow one inherent impl block per type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WriteDest<'memory> {
    start: GameAddr,
    len: usize,
    memory: PhantomData<&'memory [u8]>,
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
    /// The bytes of a write or of an offset lie past the end of their destination, so no write was
    /// tried.
    #[error("the write lies past the end of its destination")]
    OutsideDestination,
}

/// Writes `value` to the start of `dest`.
///
/// # Errors
///
/// Returns [`WriteError::OutsideDestination`] when `value` does not fit `dest`, and
/// [`WriteError::Faulted`] when any byte of the destination cannot be written. The destination may
/// then hold part of `value`.
pub fn try_write<T: Plain>(dest: WriteDest<'_>, value: T) -> Result<(), WriteError> {
    try_write_slice(dest, &[value])
}

/// Writes `values` to the start of `dest` and the bytes after it, in one guarded copy that makes no
/// heap allocation.
///
/// # Errors
///
/// Returns [`WriteError::OutsideDestination`] when `values` do not fit `dest`, and
/// [`WriteError::Faulted`] when any byte of the destination cannot be written. The destination may
/// then hold part of `values`.
pub fn try_write_slice<T: Plain>(dest: WriteDest<'_>, values: &[T]) -> Result<(), WriteError> {
    raw::write_from(dest, values)
}
