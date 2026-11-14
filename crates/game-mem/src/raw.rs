//! game-mem's unsafe edge: the guarded copy in both directions and the Windows memory queries behind
//! safe wrappers.

#![expect(unsafe_code, reason = "the guarded copy and the memory queries are foreign calls")]

use core::ffi::c_void;
use core::marker::PhantomData;
use core::ptr;

use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Memory::{
    MEM_COMMIT, MEMORY_BASIC_INFORMATION, PAGE_EXECUTE, PAGE_EXECUTE_READ, PAGE_EXECUTE_READWRITE,
    PAGE_EXECUTE_WRITECOPY, PAGE_GUARD, PAGE_NOACCESS, VirtualQuery,
};
use windows::core::PCWSTR;

use crate::{GameAddr, Plain, WriteDest, WriteError};

/// The shim's guarded copy reports a complete copy with 1.
const COPY_COMPLETE: i32 = 1;

impl WriteDest<'_> {
    /// The `len` bytes from `start` as the destination of guarded writes.
    ///
    /// # Safety
    ///
    /// For the destination's lifetime, the `len` bytes from `start` are game memory, memory the caller
    /// owns for writes or unmapped addresses, and no Rust reference to any of them is live while a
    /// write runs.
    pub const unsafe fn new(start: GameAddr, len: usize) -> Self {
        Self { start, len, memory: PhantomData }
    }

    /// The first byte of the range.
    pub const fn start(self) -> GameAddr {
        self.start
    }

    /// The part of the range from `bytes` past its start on.
    ///
    /// # Errors
    ///
    /// Returns [`WriteError::OutsideDestination`] when `bytes` exceeds the range's length.
    pub const fn offset(self, bytes: usize) -> Result<Self, WriteError> {
        let (Some(len), Some(start)) = (self.len.checked_sub(bytes), self.start.checked_add(bytes))
        else {
            return Err(WriteError::OutsideDestination);
        };
        Ok(Self { start, len, memory: PhantomData })
    }
}

pub(crate) fn read<T: Plain>(address: GameAddr) -> Option<T> {
    let mut value = T::default();
    let destination = (&raw mut value).cast::<c_void>();
    let source = ptr::with_exposed_provenance::<c_void>(address.get());
    // SAFETY: `destination` is a live `T` valid for `size_of::<T>()` byte writes. `Plain` types accept
    // every bit pattern, so even a partial copy leaves a valid `T`. The copy needs no valid `source`.
    let copied = unsafe { shim::raw::guarded_copy(destination, source, size_of::<T>()) };
    (copied == COPY_COMPLETE).then_some(value)
}

pub(crate) fn read_into<T: Plain>(address: GameAddr, values: &mut [T]) -> Option<()> {
    let destination = values.as_mut_ptr().cast::<c_void>();
    let source = ptr::with_exposed_provenance::<c_void>(address.get());
    // SAFETY: `destination` is valid for `size_of_val(values)` byte writes. `Plain` types accept every
    // bit pattern, so even a partial copy leaves valid values. The copy needs no valid `source`.
    let copied = unsafe { shim::raw::guarded_copy(destination, source, size_of_val(values)) };
    (copied == COPY_COMPLETE).then_some(())
}

/// Copies `values` to the start of `dest`. A fault may leave a partial copy in the destination.
pub(crate) fn write_from<T: Plain>(dest: WriteDest<'_>, values: &[T]) -> Result<(), WriteError> {
    let len = size_of_val(values);
    if len > dest.len {
        return Err(WriteError::OutsideDestination);
    }
    let destination = ptr::with_exposed_provenance_mut::<c_void>(dest.start.get());
    let source = values.as_ptr().cast::<c_void>();
    // SAFETY: `source` is valid for `len` byte reads. The copy needs no valid `destination`. The
    // `len` bytes lie inside `dest`, whose contract has no Rust reference cover them.
    let copied = unsafe { shim::raw::guarded_copy(destination, source, len) };
    if copied == COPY_COMPLETE {
        Ok(())
    } else {
        Err(WriteError::Faulted { address: dest.start.get() })
    }
}

/// The load address of the process exe's module.
pub(crate) fn main_module_base() -> windows::core::Result<usize> {
    // SAFETY: a null name asks for the process exe's module, which stays loaded for the process's life.
    let module = unsafe { GetModuleHandleW(PCWSTR::null()) }?;
    Ok(module.0.expose_provenance())
}

/// Whether a region's pages hold code a guarded read can copy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Access {
    Code,
    Other,
}

/// A run of pages with the same state and protection, as `VirtualQuery` reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Region {
    pub(crate) base: usize,
    pub(crate) size: usize,
    pub(crate) allocation_base: usize,
    pub(crate) access: Access,
}

impl Region {
    /// The address after the region, or `None` when the region is empty or ends the address space.
    pub(crate) fn end(&self) -> Option<usize> {
        if self.size == 0 {
            return None;
        }
        self.base.checked_add(self.size)
    }
}

/// The region that holds `address`, or `None` past the highest user-mode address.
pub(crate) fn query(address: usize) -> Option<Region> {
    let mut info = MEMORY_BASIC_INFORMATION::default();
    let size = size_of::<MEMORY_BASIC_INFORMATION>();
    let address = ptr::with_exposed_provenance::<c_void>(address);
    // SAFETY: `info` is a live MEMORY_BASIC_INFORMATION of `size` bytes, and the query reads no memory
    // at `address`.
    let written = unsafe { VirtualQuery(Some(address), &raw mut info, size) };
    if written == 0 {
        return None;
    }
    Some(Region {
        base: info.BaseAddress.expose_provenance(),
        size: info.RegionSize,
        allocation_base: info.AllocationBase.expose_provenance(),
        access: access(&info),
    })
}

fn access(info: &MEMORY_BASIC_INFORMATION) -> Access {
    let executable =
        PAGE_EXECUTE | PAGE_EXECUTE_READ | PAGE_EXECUTE_READWRITE | PAGE_EXECUTE_WRITECOPY;
    let unreadable = PAGE_GUARD | PAGE_NOACCESS;
    let committed = info.State == MEM_COMMIT;
    if committed && info.Protect.0 & executable.0 != 0 && info.Protect.0 & unreadable.0 == 0 {
        Access::Code
    } else {
        Access::Other
    }
}

#[cfg(test)]
mod tests {
    use core::ffi::c_void;

    use windows::Win32::System::Memory::{
        MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, MEMORY_BASIC_INFORMATION, PAGE_GUARD,
        PAGE_PROTECTION_FLAGS, PAGE_READONLY, PAGE_READWRITE, VirtualAlloc, VirtualFree,
        VirtualQuery,
    };

    use std::hint::black_box;

    use super::{read, write_from};
    use crate::{GameAddr, WriteDest, WriteError, try_write, try_write_slice};

    /// The x64 Windows page size, which the guarded copy's page mask assumes.
    const PAGE_SIZE: usize = 4096;

    fn protection(page: *const c_void) -> PAGE_PROTECTION_FLAGS {
        let mut info = MEMORY_BASIC_INFORMATION::default();
        let size = size_of::<MEMORY_BASIC_INFORMATION>();
        // SAFETY: `info` is a live MEMORY_BASIC_INFORMATION of `size` bytes, and the query reads no
        // memory at `page`.
        let written = unsafe { VirtualQuery(Some(page), &raw mut info, size) };
        assert_ne!(written, 0, "query the test page");
        info.Protect
    }

    // A guard page fault clears PAGE_GUARD, and a thread's stack growth needs it set again.
    #[test]
    fn a_read_of_a_guard_page_is_none_and_leaves_the_page_guarded() {
        let guarded = PAGE_READWRITE | PAGE_GUARD;
        // SAFETY: the system picks the address of a new committed page, which only this test uses.
        let page = unsafe { VirtualAlloc(None, PAGE_SIZE, MEM_COMMIT | MEM_RESERVE, guarded) };
        assert!(!page.is_null(), "commit a guard page");
        let address = GameAddr::new(page.expose_provenance());
        let first = read::<u64>(address);
        let after_first = protection(page);
        let second = read::<u64>(address);
        // SAFETY: `page` is the base of the allocation above, and this is its one release.
        unsafe { VirtualFree(page, 0, MEM_RELEASE) }.expect("release the test page");
        assert_eq!(first, None);
        assert_eq!(after_first, guarded, "the guarded copy set PAGE_GUARD again");
        assert_eq!(second, None, "a second read faults on the guard again");
    }

    #[test]
    fn a_write_to_a_read_only_page_fails_and_leaves_the_page_unchanged() {
        // SAFETY: the system picks the address of a new committed page, which only this test uses.
        let page =
            unsafe { VirtualAlloc(None, PAGE_SIZE, MEM_COMMIT | MEM_RESERVE, PAGE_READONLY) };
        assert!(!page.is_null(), "commit a read-only page");
        let address = GameAddr::new(page.expose_provenance());
        // SAFETY: the page is owned by this test until its release below, and no Rust reference
        // covers it.
        let dest = unsafe { WriteDest::new(address, PAGE_SIZE) };
        let wrote = write_from(dest, &[u64::MAX]);
        let after = read::<u64>(address);
        // SAFETY: `page` is the base of the allocation above, and this is its one release.
        unsafe { VirtualFree(page, 0, MEM_RELEASE) }.expect("release the test page");
        assert_eq!(wrote, Err(WriteError::Faulted { address: address.get() }));
        assert_eq!(after, Some(0), "the committed page still reads as zero");
    }

    /// Windows reserves the lowest 64 KiB of every process, so no write there succeeds.
    const UNMAPPED: GameAddr = GameAddr::new(0x10);

    /// A value whose eight bytes all differ, so a write of the wrong bytes shows.
    const STORED: u64 = 0x0123_4567_89AB_CDEF;

    /// Four zero words, filled only through guarded writes while `run` runs, and their final values.
    fn written_words(run: impl FnOnce(WriteDest<'_>)) -> [u64; 4] {
        let mut words = black_box([0_u64; 4]);
        let start = GameAddr::new((&raw mut words).expose_provenance());
        // SAFETY: `words` lives until this function returns, and no reference to it is live while
        // `run` writes.
        let dest = unsafe { WriteDest::new(start, size_of_val(&words)) };
        run(dest);
        black_box(words)
    }

    #[test]
    fn try_write_stores_values_at_the_start_and_an_offset_of_a_destination() {
        let written = written_words(|dest| {
            assert_eq!(try_write(dest, STORED), Ok(()));
            let second = dest.offset(size_of::<u64>()).expect("an offset inside the words");
            assert_eq!(second.start(), dest.start().checked_add(8).expect("a test address"));
            assert_eq!(try_write_slice(second, &[7_u64, 8, 9]), Ok(()));
        });
        assert_eq!(written, [STORED, 7, 8, 9]);
    }

    #[test]
    fn a_write_or_offset_past_the_destination_end_writes_nothing() {
        let written = written_words(|dest| {
            let outside = Err(WriteError::OutsideDestination);
            assert_eq!(try_write_slice(dest, &[1_u64, 2, 3, 4, 5]), outside, "five words");
            let last = dest.offset(24).expect("the last word");
            assert_eq!(try_write_slice(last, &[1_u32, 2, 3]), outside, "twelve bytes in eight");
            assert_eq!(dest.offset(33), outside.map(|()| dest), "an offset past the end");
            assert_eq!(try_write(dest.offset(32).expect("the end"), 1_u8), outside, "a byte");
        });
        assert_eq!(written, [0; 4]);
        assert_eq!(
            WriteError::OutsideDestination.to_string(),
            "the write lies past the end of its destination"
        );
    }

    #[test]
    fn try_write_to_an_unmapped_destination_yields_the_fault_and_no_crash() {
        // SAFETY: the bytes are unmapped, so no Rust reference covers them.
        let dest = unsafe { WriteDest::new(UNMAPPED, PAGE_SIZE) };
        let expected = Err(WriteError::Faulted { address: UNMAPPED.get() });
        assert_eq!(try_write(dest, STORED), expected);
        assert_eq!(try_write_slice(dest, &[1.5_f32, 2.5]), expected);
        assert_eq!(
            WriteError::Faulted { address: 0x10 }.to_string(),
            "a guarded write to 0x10 faulted, so it wrote none or part of its bytes"
        );
    }

    #[test]
    fn an_offset_that_passes_the_end_of_the_address_space_is_outside_the_destination() {
        let top = GameAddr::new(usize::MAX);
        // SAFETY: the last byte of the address space is kernel memory, never a Rust value.
        let dest = unsafe { WriteDest::new(top, 2) };
        assert_eq!(dest.offset(1).map(WriteDest::start), Err(WriteError::OutsideDestination));
    }
}
