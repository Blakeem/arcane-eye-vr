//! The core DLL's unsafe edge.
//!
//! It holds the exports, the Present call-site, view camera and eye camera hook installs, the write
//! destinations of the render view and of the pages the core shares with game threads, the launch
//! token's status field, and the D3D11 and DXGI calls of the frame handler. Its `eye_view` module holds
//! the eye view feature's hook install and game calls.

#![expect(
    unsafe_code,
    reason = "the DLL export, the hook install, the token view and the D3D11 and DXGI calls are unsafe"
)]

use std::ffi::c_void;
use std::mem::ManuallyDrop;
use std::ptr;

use game_bindings::render_view::RenderView;
use game_mem::{GameAddr, WriteDest, try_read};
use hooks::{HookError, MidHook, MidThunk, Registers};
use ipc_layout::core_init::InitFn;
use ipc_layout::launch_token::{self, STATUS_OFFSET, Status};
use windows::Win32::Foundation::{CloseHandle, ERROR_FILE_NOT_FOUND};
use windows::Win32::Graphics::Direct3D11::{ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT;
use windows::Win32::Graphics::Dxgi::IDXGISwapChain;
use windows::Win32::System::Memory::{
    FILE_MAP_READ, FILE_MAP_WRITE, MEM_COMMIT, MEM_RELEASE, MEM_RESERVE,
    MEMORY_MAPPED_VIEW_ADDRESS, MapViewOfFile, OpenFileMappingW, PAGE_READWRITE, UnmapViewOfFile,
    VirtualAlloc, VirtualFree,
};
use windows::core::{HRESULT, HSTRING, Interface as _};

/// The byte offset of `IDXGISwapChain::GetDesc` in the swapchain's vtable, the highest slot the
/// frame handler calls: `IUnknown` 0 to 2, `IDXGIObject` 3 to 6, `GetDevice` 7, `GetDesc` 12.
const GET_DESC_SLOT_OFFSET: usize = 12 * size_of::<usize>();

/// Starts the core: the root folder, the session log, the settings files, the Present call-site, view
/// camera, eye camera and eye view hooks, then the core context. `mode` is the launch token's mode
/// value.
///
/// Returns 0 on success and an [`InitStatus`](crate::init::InitStatus) failure code otherwise. A mode
/// value that names no mode returns the unknown-mode code, and a second call in one process returns
/// the already-running code. Neither starts anything. A root folder that cannot be found returns the
/// root-failed code before any log exists. A hook that does not install is no failure here.
/// The launch token's status reports it, and the game runs unmodified.
#[unsafe(no_mangle)]
pub(crate) extern "system" fn arcane_eye_init(mode: u32) -> u32 {
    crate::init::init_process(mode)
}

// The proxy calls the export through `InitFn`, so a signature that differs fails the build.
const _: InitFn = arcane_eye_init;

/// Installs `thunk` as the mid hook at the Present call site `address`.
///
/// `address` must be the one match of the `dx11_present_call` signature in the loaded game exe. The
/// hook never drops, so it stays installed for the process life on every path. The engine's Present
/// returns into the trampoline that holds the moved `call`, and a drop frees that trampoline.
pub(crate) fn install_present_hook(
    address: GameAddr,
    thunk: MidThunk,
) -> Result<ManuallyDrop<MidHook>, HookError> {
    // SAFETY: the signature's one match starts with `mov rax,[rcx]` and `call [rax+40h]`, 6 bytes that
    // SafetyHook's 5-byte jump relocates. Its 14-byte fallback also moves `mov esi,eax` and
    // `lea ecx,[rax+7785FFFBh]`, neither RIP-relative. A cdb disassembly of `FlipBuffers` shows no
    // branch to bytes 1 to 13 of the match, and its FH4 data holds no try block, so no catch resumes
    // there. bg3-vr hooks the same address. `FlipBuffers` runs on the render thread, which the render
    // thunk's token requires.
    let hook = unsafe { hooks::install_mid_hook(address, thunk) }?;
    Ok(ManuallyDrop::new(hook))
}

/// Installs `thunk` as the mid hook at the camera-built site `address`.
///
/// `address` must be the one match of the `render_view_camera_built` signature in the loaded game exe.
/// The hook never drops, so it stays installed for the process life on every path, and no render
/// thread runs a freed stub.
pub(crate) fn install_view_camera_hook(
    address: GameAddr,
    thunk: MidThunk,
) -> Result<ManuallyDrop<MidHook>, HookError> {
    // SAFETY: the signature's one match starts with `lea rdx,[rdi+2340h]`, a 7-byte instruction with no
    // RIP-relative operand. SafetyHook's 5-byte jump moves only it, so no jump targets a moved byte past
    // the match start. Its 14-byte fallback rejects the RIP-relative `call` at byte 12 and installs
    // nothing. bg3-vr hooks the same address. Begin-recording runs on the render thread, which the
    // render thunk's token requires.
    let hook = unsafe { hooks::install_mid_hook(address, thunk) }?;
    Ok(ManuallyDrop::new(hook))
}

/// Installs `thunk` as the mid hook at the begin-recording entry `address`.
///
/// `address` must be the one match of the `render_view_begin` signature in the loaded game exe. The
/// hook never drops, so it stays installed for the process life on every path, and no render thread
/// runs a freed stub.
pub(crate) fn install_eye_camera_hook(
    address: GameAddr,
    thunk: MidThunk,
) -> Result<ManuallyDrop<MidHook>, HookError> {
    // SAFETY: the signature's one match is a function entry that starts with `push rbx` and
    // `sub rsp,20h`, 6 bytes with no RIP-relative operand that SafetyHook's 5-byte jump moves, and no
    // jump targets a byte past a function's entry. bg3-vr hooks the same address. Begin-recording runs
    // on the render thread, which the render thunk's token requires.
    let hook = unsafe { hooks::install_mid_hook(address, thunk) }?;
    Ok(ManuallyDrop::new(hook))
}

/// The render view that `rcx` holds at the begin-recording entry, and the destination of the core's
/// writes to its first [`RenderView::WRITE_LEN`] bytes, which lives for the hook's borrow of
/// `registers`. Only the eye camera hook passes its registers here.
pub(crate) fn begin_recording_view(registers: &Registers) -> (RenderView, WriteDest<'_>) {
    let address = GameAddr::new(registers.rcx());
    // SAFETY: at the begin-recording entry `rcx` holds the engine's render view, game memory that no
    // Rust reference covers. The destination lives only for the hook, while the engine records it.
    let writes = unsafe { WriteDest::new(address, RenderView::WRITE_LEN) };
    (RenderView::new(address), writes)
}

/// Committed, zero-filled read-write pages that the core shares with game threads, such as the eye
/// camera's proxy controllers. The pages are released on drop.
///
/// Only guarded copies reach the pages, so no Rust reference covers them while a game thread reads or
/// writes them. The address is a plain number, so the core context that holds the pages stays
/// `Send + Sync`.
#[derive(Debug)]
pub(crate) struct SharedPages {
    address: usize,
    len: usize,
}

impl SharedPages {
    /// Commits pages that hold at least `len` bytes.
    ///
    /// # Errors
    ///
    /// Returns the Windows error of a failed commit, such as no memory left.
    pub(crate) fn commit(len: usize) -> Result<Self, windows::core::Error> {
        let allocation = MEM_COMMIT | MEM_RESERVE;
        // SAFETY: a null address lets the system place the new pages, which only this value owns.
        let pages = unsafe { VirtualAlloc(None, len, allocation, PAGE_READWRITE) };
        if pages.is_null() {
            return Err(windows::core::Error::from_thread());
        }
        Ok(Self { address: pages.expose_provenance(), len })
    }

    /// The first byte of the pages, which is page-aligned.
    pub(crate) const fn address(&self) -> GameAddr {
        GameAddr::new(self.address)
    }

    /// The `len` bytes `commit` was given, as the destination of guarded writes while the pages live.
    pub(crate) const fn destination(&self) -> WriteDest<'_> {
        // SAFETY: the pages stay committed until this value drops, which outlasts the borrow, and only
        // guarded copies reach them, so no Rust reference covers them.
        unsafe { WriteDest::new(self.address(), self.len) }
    }
}

impl Drop for SharedPages {
    fn drop(&mut self) {
        let pages = ptr::with_exposed_provenance_mut::<c_void>(self.address);
        // A failed release leaves these pages committed, and the core context that holds them lives
        // for the process.
        // SAFETY: `address` is the base of the pages `commit` committed, and only this drop releases
        // them.
        drop(unsafe { VirtualFree(pages, 0, MEM_RELEASE) });
    }
}

/// The back buffer's size and format, as the swapchain describes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct BackBuffer {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) format: DXGI_FORMAT,
}

/// The swapchain's device and its immediate context. Each holds a reference until it drops.
pub(crate) type Devices = (ID3D11Device, ID3D11DeviceContext);

/// Runs `f` with the swapchain that `rcx` holds at the Present call site, and its address.
///
/// Returns `None` when guarded reads find no readable vtable with a nonzero `GetDesc` slot at that
/// address. The reads reject a bad pointer, not another object. The signature match at the Present
/// call site makes `rcx` a swapchain. Only the Present call-site hook passes its registers here.
pub(crate) fn with_present_swapchain<R>(
    registers: &Registers,
    f: impl FnOnce(usize, &IDXGISwapChain) -> R,
) -> Option<R> {
    // SAFETY: at the Present call site `rcx` holds the swapchain the engine calls Present on, which
    // lives for the duration of the hook.
    unsafe { with_swapchain(registers.rcx(), f) }
}

/// Runs `f` with the swapchain at `address`, after guarded reads find a readable vtable with a
/// nonzero `GetDesc` slot there.
///
/// # Safety
///
/// `address` is unreadable or holds a live `IDXGISwapChain` until `f` returns.
unsafe fn with_swapchain<R>(
    address: usize,
    f: impl FnOnce(usize, &IDXGISwapChain) -> R,
) -> Option<R> {
    let vtable = try_read::<usize>(GameAddr::new(address))?;
    let slot = GameAddr::new(vtable).checked_add(GET_DESC_SLOT_OFFSET)?;
    try_read::<usize>(slot).filter(|function| *function != 0)?;
    let raw = ptr::with_exposed_provenance_mut::<c_void>(address);
    // SAFETY: the caller guarantees a live swapchain at `address`, and `raw` outlives the borrow.
    let swapchain = unsafe { IDXGISwapChain::from_raw_borrowed(&raw) }?;
    Some(f(address, swapchain))
}

// The per-frame calls below return the bare HRESULT, since `windows::core::Error` is not `Copy` and
// holds a COM error-info reference.

pub(crate) fn back_buffer(swapchain: &IDXGISwapChain) -> Result<BackBuffer, HRESULT> {
    // SAFETY: `swapchain` is a live swapchain, and `GetDesc` takes no reference.
    let desc = unsafe { swapchain.GetDesc() }?;
    let mode = desc.BufferDesc;
    Ok(BackBuffer { width: mode.Width, height: mode.Height, format: mode.Format })
}

/// The swapchain's back buffer. The result holds a reference, which must drop before the frame
/// handler returns, or the game's `ResizeBuffers` fails.
pub(crate) fn back_buffer_texture(swapchain: &IDXGISwapChain) -> Result<ID3D11Texture2D, HRESULT> {
    // SAFETY: `swapchain` is a live swapchain. `GetBuffer` adds the reference the result holds.
    let texture = unsafe { swapchain.GetBuffer::<ID3D11Texture2D>(0) }?;
    Ok(texture)
}

pub(crate) fn device(swapchain: &IDXGISwapChain) -> Result<ID3D11Device, HRESULT> {
    // SAFETY: `swapchain` is a live swapchain. `GetDevice` adds the reference the result holds.
    let device = unsafe { swapchain.GetDevice::<ID3D11Device>() }?;
    Ok(device)
}

pub(crate) fn immediate_context(device: &ID3D11Device) -> Result<ID3D11DeviceContext, HRESULT> {
    // SAFETY: `device` is a live device. The call adds the reference the result holds.
    let context = unsafe { device.GetImmediateContext() }?;
    Ok(context)
}

/// A read-write view of the launch token, through which the core reports its progress. The view is
/// unmapped on drop.
///
/// The address is a plain number, so the core context that holds the view stays `Send + Sync`.
#[derive(Debug)]
pub(crate) struct StatusView {
    address: usize,
}

/// Why the launch token could not be opened or mapped, for any cause but a missing mapping.
#[derive(Debug, thiserror::Error)]
pub(crate) enum StatusViewError {
    #[error("opening the launch token's mapping failed: {0}")]
    Open(#[source] windows::core::Error),
    #[error("mapping a view of the launch token failed: {0}")]
    Map(#[source] windows::core::Error),
}

impl StatusView {
    /// Maps the token in the mapping `name`, or returns `None` when no such mapping exists. The
    /// launch app may have exited, so a missing mapping is no error.
    ///
    /// # Errors
    ///
    /// Returns [`StatusViewError`] when opening fails for another cause, such as access denied, or
    /// when mapping the view fails.
    pub(crate) fn open(name: &str) -> Result<Option<Self>, StatusViewError> {
        let name = HSTRING::from(name);
        let access = FILE_MAP_READ | FILE_MAP_WRITE;
        // SAFETY: `name` is a NUL-terminated wide string that outlives the call.
        let mapping = match unsafe { OpenFileMappingW(access.0, false, &name) } {
            Ok(mapping) => mapping,
            Err(error) if error.code() == ERROR_FILE_NOT_FOUND.to_hresult() => return Ok(None),
            Err(error) => return Err(StatusViewError::Open(error)),
        };
        // SAFETY: `mapping` is the open mapping handle above, and the view spans the token's bytes.
        let view = unsafe { MapViewOfFile(mapping, access, 0, 0, launch_token::SIZE) };
        // The error is read before the close, which can overwrite the thread's last error.
        let map_error = view.Value.is_null().then(windows::core::Error::from_thread);
        // The view keeps the mapping alive, so the handle closes at once. A failed close leaks one
        // handle and changes nothing else.
        // SAFETY: `mapping` is a handle this function opened and closes once.
        drop(unsafe { CloseHandle(mapping) });
        match map_error {
            Some(error) => Err(StatusViewError::Map(error)),
            None => Ok(Some(Self { address: view.Value.expose_provenance() })),
        }
    }

    /// Writes `status` to the token's status field. The write is volatile, since the launch app reads
    /// the field from another process.
    pub(crate) fn write(&self, status: Status) {
        let field =
            ptr::with_exposed_provenance_mut::<u32>(self.address).wrapping_byte_add(STATUS_OFFSET);
        // SAFETY: `STATUS_OFFSET` is a 4-aligned field offset inside the 64-byte view, and the view
        // starts on a page boundary, so `field` is in bounds and aligned.
        unsafe { ptr::write_volatile(field, status as u32) };
    }
}

impl Drop for StatusView {
    fn drop(&mut self) {
        let view =
            MEMORY_MAPPED_VIEW_ADDRESS { Value: ptr::with_exposed_provenance_mut(self.address) };
        // A failed unmap leaves this one view mapped, and the core context that holds the view lives
        // for the process.
        // SAFETY: `address` is the base of the view `open` mapped, and only this drop unmaps it.
        drop(unsafe { UnmapViewOfFile(view) });
    }
}

pub(crate) mod eye_view;

#[cfg(test)]
pub(crate) mod test_support;

#[cfg(test)]
mod tests;
