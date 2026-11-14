//! Guarded reads and writes and the main module's code ranges, run against the test binary's own
//! memory.

#[cfg(test)]
mod tests {
    use std::hint::black_box;

    use game_mem::module::main_module;
    use game_mem::{
        GameAddr, WriteError, try_read, try_read_array, try_read_bytes, try_write, try_write_slice,
    };

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
    fn try_read_array_returns_the_stored_values_and_rejects_unmapped_ones() {
        let stored = black_box([1.5_f32, -2.0, 3.25]);
        let read =
            try_read_array::<f32, 3>(address_of(&stored)).map(|values| values.map(f32::to_bits));
        assert_eq!(read, Some([1.5_f32.to_bits(), (-2.0_f32).to_bits(), 3.25_f32.to_bits()]));
        assert_eq!(try_read_array::<u64, 2>(UNMAPPED), None);
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

    #[test]
    fn try_write_stores_values_at_a_mapped_address() {
        let mut buffer = black_box(vec![0_u64; 4]);
        let address = GameAddr::new(buffer.as_mut_ptr().expose_provenance());
        assert_eq!(try_write(address, STORED), Ok(()));
        let second = address.checked_add(size_of::<u64>()).expect("a test address");
        assert_eq!(try_write_slice(second, &[7_u64, 8, 9]), Ok(()));
        let written = try_read_array::<u64, 4>(address);
        assert_eq!(written, Some([STORED, 7, 8, 9]));
        drop(black_box(buffer));
    }

    #[test]
    fn try_write_to_an_unmapped_address_yields_the_fault_and_no_crash() {
        let expected = Err(WriteError::Faulted { address: UNMAPPED.get() });
        assert_eq!(try_write(UNMAPPED, STORED), expected);
        assert_eq!(try_write_slice(UNMAPPED, &[1.5_f32, 2.5]), expected);
        assert_eq!(
            WriteError::Faulted { address: 0x10 }.to_string(),
            "a guarded write to 0x10 faulted, so it wrote none or part of its bytes"
        );
    }

    #[test]
    fn a_write_destination_is_the_offset_address_or_past_the_address_space() {
        assert_eq!(UNMAPPED.destination(8), Ok(GameAddr::new(0x18)));
        let top = GameAddr::new(usize::MAX);
        assert_eq!(top.destination(1), Err(WriteError::PastAddressSpace));
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
