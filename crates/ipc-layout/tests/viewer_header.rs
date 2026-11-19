//! Viewer header encode and decode against its word layout.

#[cfg(test)]
mod tests {
    use ipc_layout::viewer_header::pose::{FrameTag, HeadPose, HeadSample};
    use ipc_layout::viewer_header::{
        Acquire, AdapterLuid, EYE_DISTANCE_OFFSET, EyeRect, FRAME_ASPECT_OFFSET,
        FRAME_FOV_Y_OFFSET, FRAME_HEAD_SEQUENCE_OFFSET, FRAME_OFFSET, GENERATION_OFFSET,
        HEAD_ORIENTATION_OFFSET, HEAD_POSITION_OFFSET, HEAD_SEQUENCE_OFFSET, HEARTBEAT_OFFSET,
        HeaderError, PRESENT_OFFSET, SIZE, TextureInfo, ViewerHeader, WIDTH_OFFSET, WORDS,
    };

    /// A LUID low part whose four bytes differ, so a swapped byte order shows.
    const LUID_LOW: u32 = 0x1234_5678;
    /// -2 as the bits of a `u32`, which shows the high part keeps its sign.
    const LUID_HIGH_BITS: u32 = 0xFFFF_FFFE;
    /// A frame counter whose four bytes differ.
    const FRAME: u32 = 0x0102_0304;
    /// A present number whose four bytes differ from each other and from the frame counter's.
    const PRESENT: u32 = 0x0506_0708;

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
        assert_eq!(PRESENT_OFFSET, 72);
        assert_eq!(HEAD_SEQUENCE_OFFSET, 76);
        assert_eq!(HEAD_POSITION_OFFSET, 80);
        assert_eq!(HEAD_ORIENTATION_OFFSET, 92);
        assert_eq!(EYE_DISTANCE_OFFSET, 108);
        assert_eq!(FRAME_HEAD_SEQUENCE_OFFSET, 112);
        assert_eq!(FRAME_FOV_Y_OFFSET, 116);
        assert_eq!(FRAME_ASPECT_OFFSET, 120);
    }

    #[test]
    fn encode_writes_each_field_at_its_layout_word() {
        let words = ViewerHeader::new(texture(), 7, FRAME, PRESENT).encode();
        assert_eq!(words[0].to_le_bytes(), *b"B3SV");
        assert_eq!(words[1], 3, "layout version 3");
        assert_eq!(words[2], LUID_LOW, "adapter LUID low part");
        assert_eq!(words[3], LUID_HIGH_BITS, "adapter LUID high part -2");
        assert_eq!(words[4..7], [1920, 1080, 28], "width, height and format");
        assert_eq!(words[7..11], [1, 2, 3, 4], "left eye x, y, width, height");
        assert_eq!(words[11..15], [0, 0, 1920, 1080], "right eye covers the entire image");
        assert_eq!(words[15], 7, "generation");
        assert_eq!(words[16], FRAME, "frame");
        assert_eq!(words[17], 0, "heartbeat");
        assert_eq!(words[18], PRESENT, "present number");
        assert_eq!(
            words[19], 0,
            "the head sequence of a header the core made, which means no pose"
        );
        assert_eq!(words[20..], [0; 12], "head pose, frame tag and reserved");
    }

    /// The tag of a frame with the head sequence 6, 0.96 radians of vertical field of view and the
    /// aspect 16:9.
    const TAG: FrameTag = FrameTag { head_sequence: 6, fov_y_rad: 0.96, aspect: 1.777_777_8 };

    #[test]
    fn encode_writes_the_frame_tag_after_the_head_pose() {
        let words = ViewerHeader::new(texture(), 7, FRAME, PRESENT).with_frame_tag(TAG).encode();
        assert_eq!(words[28..31], [6, 0.96_f32.to_bits(), 1.777_777_8_f32.to_bits()]);
        assert_eq!(words[19..28], [0; 9], "no head pose");
        assert_eq!(words[31], 0, "reserved");
    }

    #[test]
    fn decode_reads_back_what_encode_wrote_and_the_heartbeat_the_viewer_wrote() {
        let header = ViewerHeader::new(texture(), 3, 99, 120).with_frame_tag(TAG);
        let mut words = header.encode();
        assert_eq!(ViewerHeader::decode(&words), Ok(header));
        words[17] = 41;
        let decoded = ViewerHeader::decode(&words).expect("a valid header");
        assert_eq!(decoded.heartbeat(), 41);
        assert_eq!(decoded.texture(), texture());
        assert_eq!(decoded.generation(), 3);
        assert_eq!(decoded.frame(), 99);
        assert_eq!(decoded.present(), 120);
        assert_eq!(decoded.frame_tag(), TAG);
        assert_eq!(decoded.head().sequence(), 0);
    }

    #[test]
    fn decode_rejects_a_new_mapping_and_another_layout_version() {
        let zeroed = ViewerHeader::decode(&[0; WORDS]);
        assert_eq!(zeroed, Err(HeaderError::Magic { found: 0 }));
        let mut words = ViewerHeader::new(TextureInfo::default(), 0, 0, 0).encode();
        words[1] = 2;
        let error = ViewerHeader::decode(&words).expect_err("layout version 2");
        assert_eq!(error, HeaderError::LayoutVersion { found: 2 });
        assert_eq!(error.to_string(), "the viewer header's layout version is 2, not 3");
    }

    #[test]
    fn core_words_cover_every_word_but_the_heartbeat_and_the_head_pose_in_ascending_order() {
        let header = ViewerHeader::new(texture(), 5, 6, 8).with_frame_tag(TAG);
        let words = header.encode();
        let written: Vec<(usize, u32)> = header.core_words().collect();
        assert_eq!(written.len(), WORDS - 10, "the heartbeat and the 9 head pose words left out");
        let viewer_owned =
            |offset: usize| offset == HEARTBEAT_OFFSET || (76..112).contains(&offset);
        assert!(written.iter().all(|(offset, _word)| !viewer_owned(*offset)), "{written:?}");
        assert!(written.is_sorted_by_key(|(offset, _word)| *offset), "{written:?}");
        for (offset, word) in &written {
            assert_eq!(Some(word), words.get(offset / 4), "the word at offset {offset}");
        }
        let generation = written.iter().position(|(offset, _word)| *offset == GENERATION_OFFSET);
        assert_eq!(generation, Some(15), "the generation follows every texture word");
    }

    /// Applies the viewer's writes of `pose` with the eye distance 0.064 m to `words` and returns
    /// the sequence number they end with.
    fn write_pose(words: &mut [u32; WORDS], pose: HeadPose) -> u32 {
        let header = ViewerHeader::decode(words).expect("a valid header");
        let (sequence, writes) = header.head().writes(pose, 0.064);
        for (offset, value) in writes {
            words[offset / 4] = value;
        }
        sequence
    }

    /// The head pose a reader takes from `words`, with the sequence number read again unchanged.
    fn read_pose(words: &[u32; WORDS]) -> Option<HeadSample> {
        let head = ViewerHeader::decode(words).expect("a valid header").head();
        head.sample(head.sequence())
    }

    #[test]
    fn pose_writes_end_on_2_then_4_and_a_read_gives_the_last_pose_with_its_number() {
        let mut words = ViewerHeader::new(texture(), 1, 1, 1).encode();
        assert_eq!(read_pose(&words), None, "a header the core made holds no pose");
        let first = HeadPose { position: [0.0, 1.5, 0.0], orientation: [0.0, 0.0, 0.0, 1.0] };
        assert_eq!(write_pose(&mut words, first), 2, "the first write");
        assert_eq!(words[19], 2);
        let turned = [0.0, -0.173_648_18, 0.0, 0.984_807_7];
        let second = HeadPose { position: [0.1, 1.6, -0.2], orientation: turned };
        assert_eq!(write_pose(&mut words, second), 4, "the second write");
        assert_eq!(words[19], 4);
        let expected = HeadSample { sequence: 4, pose: second, eye_distance_m: 0.064 };
        assert_eq!(read_pose(&words), Some(expected));
        words[19] = 3;
        assert_eq!(read_pose(&words), None, "a header whose head sequence is 3");
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
