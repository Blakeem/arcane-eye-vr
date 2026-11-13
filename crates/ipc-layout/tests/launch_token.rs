//! Launch token encode and decode against a byte layout written out by hand.

#[cfg(test)]
mod tests {
    use ipc_layout::launch_token::{
        GAME_PID_OFFSET, LaunchToken, Mode, STATUS_OFFSET, Status, TokenError, UnknownMode,
        UnknownStatus,
    };

    #[test]
    fn encode_writes_each_field_at_its_layout_offset() {
        let launch_app_pid = u32::from_le_bytes([4, 3, 2, 1]);
        let created = u64::from_le_bytes([0x18, 0x17, 0x16, 0x15, 0x14, 0x13, 0x12, 0x11]);
        let token = LaunchToken::new(Mode::Vr, launch_app_pid, created);
        let bytes = token.encode();
        assert_eq!(&bytes[0..4], b"B3SL");
        assert_eq!(&bytes[4..8], &[1, 0, 0, 0], "layout version 1");
        assert_eq!(&bytes[8..12], &[2, 0, 0, 0], "mode VR");
        assert_eq!(&bytes[12..16], &[4, 3, 2, 1], "launch app pid");
        assert_eq!(&bytes[16..24], &[0x18, 0x17, 0x16, 0x15, 0x14, 0x13, 0x12, 0x11]);
        assert_eq!(&bytes[24..28], &[0, 0, 0, 0], "status waiting");
        assert_eq!(&bytes[28..32], &[0, 0, 0, 0], "no game pid yet");
        assert_eq!(&bytes[32..64], &[0; 32], "reserved");
        assert_eq!(STATUS_OFFSET, 24);
        assert_eq!(GAME_PID_OFFSET, 28);
    }

    #[test]
    fn decode_reads_back_what_encode_wrote() {
        let token = LaunchToken::new(Mode::DesktopStereo, 4242, 1_700_000_000_000);
        let decoded = LaunchToken::decode(&token.encode()).expect("a valid token");
        assert_eq!(decoded, token);
        assert_eq!(decoded.mode(), Ok(Mode::DesktopStereo));
        assert_eq!(decoded.raw_mode(), 1);
        assert_eq!(decoded.status(), Ok(Status::Waiting));
        assert_eq!(decoded.game_pid(), 0);
    }

    #[test]
    fn decode_reads_status_and_game_pid_written_at_their_offsets() {
        let mut bytes = LaunchToken::new(Mode::Vr, 1, 2).encode();
        bytes[24..28].copy_from_slice(&11u32.to_le_bytes());
        bytes[28..32].copy_from_slice(&9876u32.to_le_bytes());
        let decoded = LaunchToken::decode(&bytes).expect("a valid token");
        assert_eq!(decoded.status(), Ok(Status::CoreLoadFailed));
        assert_eq!(decoded.game_pid(), 9876);
    }

    #[test]
    fn decode_rejects_a_wrong_magic_value() {
        let mut bytes = LaunchToken::new(Mode::Vr, 1, 2).encode();
        bytes[0] = b'X';
        let expected = u32::from_le_bytes(*b"X3SL");
        assert_eq!(LaunchToken::decode(&bytes), Err(TokenError::Magic { found: expected }));
    }

    #[test]
    fn decode_rejects_another_layout_version() {
        let mut bytes = LaunchToken::new(Mode::Vr, 1, 2).encode();
        bytes[4] = 2;
        assert_eq!(LaunchToken::decode(&bytes), Err(TokenError::LayoutVersion { found: 2 }));
    }

    #[test]
    fn decode_rejects_a_block_that_is_not_64_bytes() {
        let bytes = LaunchToken::new(Mode::Vr, 1, 2).encode();
        assert_eq!(LaunchToken::decode(&bytes[..63]), Err(TokenError::Size { len: 63 }));
        let mut long = bytes.to_vec();
        long.push(0);
        assert_eq!(LaunchToken::decode(&long), Err(TokenError::Size { len: 65 }));
        assert_eq!(LaunchToken::decode(&[]), Err(TokenError::Size { len: 0 }));
    }

    #[test]
    fn an_all_zero_block_is_not_a_token() {
        assert_eq!(LaunchToken::decode(&[0; 64]), Err(TokenError::Magic { found: 0 }));
    }

    #[test]
    fn mode_decode_accepts_only_the_two_modes() {
        assert_eq!(Mode::try_from(1), Ok(Mode::DesktopStereo));
        assert_eq!(Mode::try_from(2), Ok(Mode::Vr));
        assert_eq!(Mode::try_from(0), Err(UnknownMode(0)));
        assert_eq!(Mode::try_from(3), Err(UnknownMode(3)));
        assert_eq!(Mode::try_from(u32::MAX), Err(UnknownMode(u32::MAX)));
        assert_eq!(Mode::DesktopStereo.as_str(), "desktop-stereo");
        assert_eq!(Mode::Vr.as_str(), "vr");
    }

    #[test]
    fn status_decode_maps_each_documented_code() {
        let documented = [
            (0, Status::Waiting),
            (1, Status::CoreLoading),
            (2, Status::HooksInstalled),
            (3, Status::FirstPresent),
            (4, Status::VrSessionRunning),
            (10, Status::WrongRenderer),
            (11, Status::CoreLoadFailed),
            (12, Status::SignatureFailure),
        ];
        for (code, status) in documented {
            assert_eq!(Status::try_from(code), Ok(status), "code {code}");
        }
        for code in [5, 9, 13, u32::MAX] {
            assert_eq!(Status::try_from(code), Err(UnknownStatus(code)), "code {code}");
        }
    }

    #[test]
    fn a_token_with_an_unknown_mode_still_decodes_and_reports_the_mode() {
        let mut bytes = LaunchToken::new(Mode::Vr, 1, 2).encode();
        bytes[8] = 7;
        let decoded = LaunchToken::decode(&bytes).expect("magic and version are valid");
        assert_eq!(decoded.raw_mode(), 7);
        assert_eq!(decoded.mode(), Err(UnknownMode(7)));
    }
}
