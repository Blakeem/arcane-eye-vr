//! The session log's quoting rule, and the log line messages the parsers find by it.

#[cfg(test)]
mod tests {
    use ipc_layout::log_fields::is_written_quoted;
    use ipc_layout::{camera_line, game_state_line, perf_line};

    #[test]
    fn an_empty_value_or_one_with_a_space_quote_equals_backslash_or_control_is_quoted() {
        for value in ["", "a b", "a\"b", "a=b", r"a\b", "a\tb", "a\u{1B}b"] {
            assert!(is_written_quoted(value), "{value:?}");
        }
        for value in ["camera", "1000.0", "-1e-45", "\u{e9}"] {
            assert!(!is_written_quoted(value), "{value:?}");
        }
    }

    #[test]
    fn no_parsed_message_holds_a_character_the_session_log_escapes() {
        for message in [camera_line::MESSAGE, game_state_line::MESSAGE, perf_line::MESSAGE] {
            let escaped = message.chars().any(|c| c == '"' || c == '\\' || c.is_control());
            assert!(!escaped, "{message:?}");
        }
    }
}
