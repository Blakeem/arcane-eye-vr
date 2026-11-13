//! The shim's C ABI: the functions `csrc/` exports and the data layouts they pass.

#![expect(unsafe_code, reason = "the shim's C ABI is a block of foreign functions")]

use core::ffi::c_void;
use core::mem::offset_of;

/// One XMM register as SafetyHook's `Xmm` union stores it.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Xmm(pub [u64; 2]);

/// SafetyHook's `Context64`: the registers at a mid hook's target when the hook runs.
///
/// A write to a field changes that register when the target resumes. SafetyHook ignores a write to
/// `rsp`.
#[repr(C)]
#[derive(Debug)]
pub struct Context {
    pub xmm: [Xmm; 16],
    pub rflags: usize,
    pub r15: usize,
    pub r14: usize,
    pub r13: usize,
    pub r12: usize,
    pub r11: usize,
    pub r10: usize,
    pub r9: usize,
    pub r8: usize,
    pub rdi: usize,
    pub rsi: usize,
    pub rdx: usize,
    pub rcx: usize,
    pub rbx: usize,
    pub rax: usize,
    pub rbp: usize,
    pub rsp: usize,
    pub trampoline_rsp: usize,
    pub rip: usize,
}

// `csrc/mid_hook.cpp` asserts the same size and offsets against the vendored header.
const _: () = assert!(size_of::<Context>() == 408);
const _: () = assert!(offset_of!(Context, rflags) == 256);
const _: () = assert!(offset_of!(Context, rcx) == 352);
const _: () = assert!(offset_of!(Context, rip) == 400);

/// SafetyHook's `MidHookFn`. The callback runs at the target with the target's registers.
pub type MidHookFn = extern "C" fn(context: &mut Context);

/// [`mid_hook_create`] installed the hook.
pub const HOOK_OK: i32 = 0;
/// SafetyHook could not allocate the hook's stub or trampoline.
pub const HOOK_BAD_ALLOCATION: i32 = 1;
/// A C++ exception, such as `std::bad_alloc`, ended the create call.
pub const HOOK_EXCEPTION: i32 = 2;
/// The inline hook under the mid hook failed. The code is this value plus SafetyHook's
/// `InlineHook::Error` type.
pub const HOOK_BAD_INLINE_HOOK: i32 = 16;

unsafe extern "C" {
    /// Copies `len` bytes from `src` to `dst` under `__try`. Returns 1 when every byte was copied and 0
    /// when a read raised an access violation or a guard page fault. After a fault `dst` holds a partial
    /// copy.
    ///
    /// # Safety
    ///
    /// `dst` is valid for writes of `len` bytes. `src` needs no validity.
    #[link_name = "shim_guarded_copy"]
    pub fn guarded_copy(dst: *mut c_void, src: *const c_void, len: usize) -> i32;

    /// Installs a SafetyHook mid hook at `target` that calls `destination`, and on success writes the
    /// hook's handle to `out_hook`. Returns [`HOOK_OK`] or one of the other `HOOK_` codes.
    ///
    /// # Safety
    ///
    /// `target` is the first byte of an instruction in executable code, and the instructions SafetyHook
    /// relocates there are no jump target. `out_hook` is valid for one pointer write.
    #[link_name = "shim_mid_hook_create"]
    pub fn mid_hook_create(
        target: *mut c_void,
        destination: MidHookFn,
        out_hook: *mut *mut c_void,
    ) -> i32;

    /// Removes the hook and restores the target's original bytes.
    ///
    /// # Safety
    ///
    /// `hook` is a handle [`mid_hook_create`] wrote and no earlier call destroyed.
    #[link_name = "shim_mid_hook_destroy"]
    pub fn mid_hook_destroy(hook: *mut c_void);
}
