//! Viewer header encode and decode against its word layout.

#[cfg(test)]
mod tests {
    use ipc_layout::viewer_header::{
        Acquire, AdapterLuid, EyeRect, FRAME_OFFSET, GENERATION_OFFSET, HEARTBEAT_OFFSET,
        HeaderError, SIZE, TextureInfo, ViewerHeader, WIDTH_OFFSET, WORDS,
    };

    /// A LUID low part whose four bytes differ, so a swapped byte order shows.
    const LUID_LOW: u32 = 0x1234_5678;
    /// -2 as the bits of a `u32`, which shows the high part keeps its sign.
    const LUID_HIGH_BITS: u32 = 0xFFFF_FFFE;
    /// A frame counter whose four bytes differ.
    const FRAME: u32 = 0x0102_0304;

    fn texture() -> TextureInfo {
        TextureInfo {
            adapter_luid: AdapterLuid { low: LUID_LOW, high: -2 },
            width: 1920,
            height: 1080,
            format: 28,
            eyes: [EyeRect { x: 1, y: 2, width: 3, height: 4 }, EyeRect::full(1920, 1080)],
        }
    }

    #[test]
    fn layout_constants_match_the_word_layout() {
        assert_eq!(WORDS, 32);
        assert_eq!(SIZE, 128);
        assert_eq!(WIDTH_OFFSET, 16);
        assert_eq!(GENERATION_OFFSET, 60);
        assert_eq!(FRAME_OFFSET, 64);
        assert_eq!(HEARTBEAT_OFFSET, 68);
    }

    #[test]
    fn encode_writes_each_field_at_its_layout_word() {
        let words = ViewerHeader::new(texture(), 7, FRAME).encode();
        assert_eq!(words[0].to_le_bytes(), *b"B3SV");
        assert_eq!(words[1], 1, "layout version 1");
        assert_eq!(words[2], LUID_LOW, "adapter LUID low part");
        assert_eq!(words[3], LUID_HIGH_BITS, "adapter LUID high part -2");
        assert_eq!(words[4..7], [1920, 1080, 28], "width, height and format");
        assert_eq!(words[7..11], [1, 2, 3, 4], "left eye x, y, width, height");
        assert_eq!(words[11..15], [0, 0, 1920, 1080], "right eye covers the entire image");
        assert_eq!(words[15], 7, "generation");
        assert_eq!(words[16], FRAME, "frame");
        assert_eq!(words[17], 0, "heartbeat");
        assert_eq!(words[18..], [0; 14], "reserved");
    }

    #[test]
    fn decode_reads_back_what_encode_wrote_and_the_heartbeat_the_viewer_wrote() {
        let header = ViewerHeader::new(texture(), 3, 99);
        let mut words = header.encode();
        assert_eq!(ViewerHeader::decode(&words), Ok(header));
        words[17] = 41;
        let decoded = ViewerHeader::decode(&words).expect("a valid header");
        assert_eq!(decoded.heartbeat(), 41);
        assert_eq!(decoded.texture(), texture());
        assert_eq!(decoded.generation(), 3);
        assert_eq!(decoded.frame(), 99);
    }

    #[test]
    fn decode_rejects_a_new_mapping_and_another_layout_version() {
        let zeroed = ViewerHeader::decode(&[0; WORDS]);
        assert_eq!(zeroed, Err(HeaderError::Magic { found: 0 }));
        let mut words = ViewerHeader::new(TextureInfo::default(), 0, 0).encode();
        words[1] = 2;
        let error = ViewerHeader::decode(&words).expect_err("layout version 2");
        assert_eq!(error, HeaderError::LayoutVersion { found: 2 });
        assert_eq!(error.to_string(), "the viewer header's layout version is 2, not 1");
    }

    #[test]
    fn core_words_cover_every_word_but_the_heartbeat_in_ascending_order() {
        let header = ViewerHeader::new(texture(), 5, 6);
        let words = header.encode();
        let written: Vec<(usize, u32)> = header.core_words().collect();
        assert_eq!(written.len(), WORDS - 1);
        assert!(written.iter().all(|(offset, _word)| *offset != HEARTBEAT_OFFSET));
        assert!(written.is_sorted_by_key(|(offset, _word)| *offset), "{written:?}");
        for (offset, word) in &written {
            assert_eq!(Some(word), words.get(offset / 4), "the word at offset {offset}");
        }
        let generation = written.iter().position(|(offset, _word)| *offset == GENERATION_OFFSET);
        assert_eq!(generation, Some(15), "the generation follows every texture word");
    }

    const S_FALSE: i32 = 1;
    const E_FAIL: i32 = 0x8000_4005_u32.cast_signed();
    const DXGI_ERROR_DEVICE_REMOVED: i32 = 0x887A_0005_u32.cast_signed();

    #[test]
    fn each_acquire_code_maps_to_its_decision() {
        assert_eq!(Acquire::from_code(0), Acquire::Held, "S_OK");
        assert_eq!(Acquire::from_code(0x102), Acquire::Busy, "WAIT_TIMEOUT");
        assert_eq!(Acquire::from_code(0x80), Acquire::Abandoned, "WAIT_ABANDONED");
        assert_eq!(Acquire::from_code(S_FALSE), Acquire::Failed(S_FALSE));
        assert_eq!(Acquire::from_code(E_FAIL), Acquire::Failed(E_FAIL));
        let removed = Acquire::from_code(DXGI_ERROR_DEVICE_REMOVED);
        assert_eq!(removed, Acquire::Failed(DXGI_ERROR_DEVICE_REMOVED));
    }
}
