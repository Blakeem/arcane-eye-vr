// The guarded copy behind game-mem's try_read and try_write. It is C because `__try` is MSVC
// structured exception handling, which Rust has no form for.
#include <windows.h>
#include <string.h>

#define PAGE_ADDRESS_MASK (~(ULONG_PTR)0xFFF)

// Catches only an access violation or a guard page fault, so every other exception reaches the
// handlers it belongs to.
static int guarded_copy_filter(DWORD code, EXCEPTION_POINTERS* info) {
    if (code == EXCEPTION_ACCESS_VIOLATION) {
        return EXCEPTION_EXECUTE_HANDLER;
    }
    if (code == STATUS_GUARD_PAGE_VIOLATION) {
        // The fault cleared PAGE_GUARD. Setting it again keeps the owning thread's stack growth working.
        void* page = (void*)(info->ExceptionRecord->ExceptionInformation[1] & PAGE_ADDRESS_MASK);
        MEMORY_BASIC_INFORMATION region;
        DWORD old_protect;
        if (VirtualQuery(page, &region, sizeof region) != 0) {
            VirtualProtect(page, 1, region.Protect | PAGE_GUARD, &old_protect);
        }
        return EXCEPTION_EXECUTE_HANDLER;
    }
    return EXCEPTION_CONTINUE_SEARCH;
}

// Returns 1 when all `len` bytes were copied and 0 when a read or a write faulted. After a fault `dst`
// holds a partial copy.
int shim_guarded_copy(void* dst, const void* src, size_t len) {
    __try {
        memcpy(dst, src, len);
        return 1;
    } __except (guarded_copy_filter(GetExceptionCode(), GetExceptionInformation())) {
        return 0;
    }
}
