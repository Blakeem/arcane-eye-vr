//! game-mem's unsafe edge: the guarded copy in both directions and the Windows memory queries behind
//! safe wrappers.

#![expect(unsafe_code, reason = "the guarded copy and the memory queries are foreign calls")]

use core::ffi::c_void;
use core::ptr;

use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Memory::{
    MEM_COMMIT, MEMORY_BASIC_INFORMATION, PAGE_EXECUTE, PAGE_EXECUTE_READ, PAGE_EXECUTE_READWRITE,
    PAGE_EXECUTE_WRITECOPY, PAGE_GUARD, PAGE_NOACCESS, VirtualQuery,
};
use windows::core::PCWSTR;

use crate::{GameAddr, Plain};

/// The shim's guarded copy reports a complete copy with 1.
const COPY_COMPLETE: i32 = 1;

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

/// Copies `values` to `address`. Returns `false` when a byte of the destination could not be
/// written, and the destination may then hold a partial copy.
pub(crate) fn write_from<T: Plain>(address: GameAddr, values: &[T]) -> bool {
    let destination = ptr::with_exposed_provenance_mut::<c_void>(address.get());
    let source = values.as_ptr().cast::<c_void>();
    // SAFETY: `source` is valid for `size_of_val(values)` byte reads. The copy needs no valid
    // `destination`. `try_write`'s contract has `address` name memory no Rust value owns.
    let copied = unsafe { shim::raw::guarded_copy(destination, source, size_of_val(values)) };
    copied == COPY_COMPLETE
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

    use super::{read, write_from};
    use crate::GameAddr;

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
        let wrote = write_from(address, &[u64::MAX]);
        let after = read::<u64>(address);
        // SAFETY: `page` is the base of the allocation above, and this is its one release.
        unsafe { VirtualFree(page, 0, MEM_RELEASE) }.expect("release the test page");
        assert!(!wrote, "a write to a read-only page succeeded");
        assert_eq!(after, Some(0), "the committed page still reads as zero");
    }
}
