// A C ABI over SafetyHook's mid hook. No C++ exception crosses it, since an exception that leaves an
// extern "C" function ends the process.
#include "safetyhook.hpp"

#include <cstddef>
#include <cstdint>

// crates/shim/src/raw.rs mirrors safetyhook::Context. These asserts fail the build when the vendored
// layout moves.
static_assert(sizeof(safetyhook::Context) == 408);
static_assert(offsetof(safetyhook::Context, rflags) == 256);
static_assert(offsetof(safetyhook::Context, rdi) == 328);
static_assert(offsetof(safetyhook::Context, rcx) == 352);
static_assert(offsetof(safetyhook::Context, rip) == 400);

// Result codes, mirrored in crates/shim/src/raw.rs. A BAD_INLINE_HOOK error adds the inline hook's own
// error type to SHIM_HOOK_BAD_INLINE_HOOK.
enum : int32_t {
    SHIM_HOOK_OK = 0,
    SHIM_HOOK_BAD_ALLOCATION = 1,
    SHIM_HOOK_EXCEPTION = 2,
    SHIM_HOOK_BAD_INLINE_HOOK = 16,
};

static int32_t mid_hook_error_code(const safetyhook::MidHook::Error& error) {
    if (error.type == safetyhook::MidHook::Error::BAD_INLINE_HOOK) {
        return SHIM_HOOK_BAD_INLINE_HOOK + static_cast<int32_t>(error.inline_hook_error.type);
    }
    return SHIM_HOOK_BAD_ALLOCATION;
}

extern "C" int32_t shim_mid_hook_create(void* target, safetyhook::MidHookFn destination, void** out_hook) {
    try {
        auto hook = safetyhook::MidHook::create(target, destination);
        if (!hook) {
            return mid_hook_error_code(hook.error());
        }
        *out_hook = new safetyhook::MidHook(std::move(*hook));
        return SHIM_HOOK_OK;
    } catch (...) {
        return SHIM_HOOK_EXCEPTION;
    }
}

// Removes the hook and restores the target's original bytes.
extern "C" void shim_mid_hook_destroy(void* hook) {
    delete static_cast<safetyhook::MidHook*>(hook);
}
