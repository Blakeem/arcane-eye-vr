//! Guarded reads and the main module's code ranges, run against the test binary's own memory.

#[cfg(test)]
mod tests {
    use std::hint::black_box;

    use game_mem::module::main_module;
    use game_mem::{GameAddr, try_read, try_read_bytes};

    /// Windows reserves the lowest 64 KiB of every process, so no read there succeeds.
    const UNMAPPED: GameAddr = GameAddr::new(0x10);

    /// A value whose eight bytes all differ, so a read of the wrong bytes shows.
    const STORED: u64 = 0x0123_4567_89AB_CDEF;

    fn address_of<T>(value: &T) -> GameAddr {
        GameAddr::new(std::ptr::from_ref(value).expose_provenance())
    }

    #[test]
    fn try_read_returns_the_stored_value_at_a_mapped_address() {
        let stored = black_box(STORED);
        assert_eq!(try_read::<u64>(address_of(&stored)), Some(STORED));
        let float = black_box(1.5_f32);
        assert_eq!(try_read::<f32>(address_of(&float)).map(f32::to_bits), Some(1.5_f32.to_bits()));
    }

    #[test]
    fn try_read_returns_none_at_an_unmapped_address() {
        assert_eq!(try_read::<u64>(UNMAPPED), None);
        assert_eq!(try_read::<u8>(GameAddr::new(0)), None);
    }

    #[test]
    fn try_read_bytes_copies_mapped_bytes_and_rejects_unmapped_ones() {
        let stored = black_box([1_u8, 2, 3, 4, 5]);
        assert_eq!(try_read_bytes(address_of(&stored), 5), Some(vec![1, 2, 3, 4, 5]));
        assert_eq!(try_read_bytes(UNMAPPED, 5), None);
    }

    #[test]
    fn try_read_bytes_returns_none_for_a_length_no_buffer_can_hold() {
        let stored = black_box(STORED);
        assert_eq!(try_read_bytes(address_of(&stored), usize::MAX), None);
    }

    #[inline(never)]
    fn code_marker() -> u32 {
        black_box(7)
    }

    #[test]
    fn main_module_code_ranges_hold_a_function_of_the_exe() {
        let module = main_module().expect("the test exe's module");
        let marker: fn() -> u32 = code_marker;
        let function = (marker as *const ()).expose_provenance();
        let holder = module.code.iter().find(|range| {
            function.checked_sub(range.start.get()).is_some_and(|offset| offset < range.len)
        });
        let holder = holder.expect("a code range holds the function");
        assert!(holder.start >= module.base, "a code range starts below the module base");
        let copy = try_read_bytes(holder.start, holder.len).expect("a code range is readable");
        assert_eq!(copy.len(), holder.len);
        assert_eq!(code_marker(), 7);
    }
}
