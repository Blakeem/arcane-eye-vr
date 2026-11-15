//! Game state line parsing against session log lines written by hand.

#[cfg(test)]
mod tests {
    use ipc_layout::game_state_line::{GameStateLine, GameStateLineError, Place, parse_line};
    use ipc_layout::log_fields::is_written_quoted;

    const PREFIX: &str = "2026-11-14T09:12:03.418 INFO  thread=29780 arcane_eye_core::game_state";

    fn game_state_line(fields: &str) -> String {
        format!("{PREFIX} msg=\"game state\" {fields}")
    }

    #[test]
    fn a_line_of_each_place_yields_its_place_and_state_id() {
        let cases =
            [("menu", 7, Place::Menu), ("level", 18, Place::Level), ("other", 10, Place::Other)];
        for (name, state_id, place) in cases {
            let line = game_state_line(&format!("place={name} state_id={state_id}"));
            assert_eq!(parse_line(&line), Ok(Some(GameStateLine { place, state_id })), "{name}");
            assert_eq!(place.as_str(), name);
            assert!(!is_written_quoted(name), "the session log writes {name} bare");
        }
    }

    #[test]
    fn a_line_ending_and_an_unknown_field_are_ignored() {
        let line = game_state_line("frame=12 place=level state_id=18\r\n");
        let expected = GameStateLine { place: Place::Level, state_id: 18 };
        assert_eq!(parse_line(&line), Ok(Some(expected)));
    }

    #[test]
    fn a_line_of_another_message_yields_nothing() {
        let perf = format!("{PREFIX} msg=\"perf summary\" frames=600");
        let longer_message = format!("{PREFIX} msg=\"game states\" place=level state_id=18");
        let bare_message = format!("{PREFIX} msg=game place=level state_id=18");
        assert_eq!(parse_line(&perf), Ok(None));
        assert_eq!(parse_line(&longer_message), Ok(None));
        assert_eq!(parse_line(&bare_message), Ok(None));
        assert_eq!(parse_line(""), Ok(None));
    }

    #[test]
    fn a_line_with_a_missing_field_names_the_field() {
        let no_place = parse_line(&game_state_line("state_id=18")).expect_err("no place");
        assert_eq!(no_place, GameStateLineError::MissingField { field: "place" });
        assert_eq!(no_place.to_string(), "the game state line has no place field");
        let no_id = parse_line(&game_state_line("place=level")).expect_err("no state id");
        assert_eq!(no_id, GameStateLineError::MissingField { field: "state_id" });
    }

    #[test]
    fn a_line_with_a_state_id_that_is_not_a_u32_names_the_value() {
        for value in ["0x12", "-1", "4294967296", ""] {
            let line = game_state_line(&format!("place=level state_id={value}"));
            let error = parse_line(&line).expect_err("a state id that is not a u32");
            assert!(
                matches!(&error, GameStateLineError::NotAStateId { value: held, .. } if held == value),
                "{error:?}"
            );
            let message =
                format!("the game state field state_id holds {value:?}, which is not a u32");
            assert_eq!(error.to_string(), message);
        }
    }

    #[test]
    fn a_line_with_an_unknown_place_names_the_value() {
        let line = game_state_line("place=Level state_id=18");
        let error = parse_line(&line).expect_err("a place name in the wrong case");
        assert_eq!(error, GameStateLineError::UnknownPlace { value: "Level".to_owned() });
        assert_eq!(
            error.to_string(),
            "the game state field place holds \"Level\", which is not menu, level or other"
        );
    }

    #[test]
    fn a_line_with_a_repeated_field_is_rejected() {
        let line = game_state_line("place=level state_id=18 state_id=7");
        let error = parse_line(&line).expect_err("a repeated field");
        assert_eq!(error, GameStateLineError::RepeatedField { field: "state_id" });
    }
}
