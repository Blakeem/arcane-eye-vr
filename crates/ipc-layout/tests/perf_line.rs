//! Perf summary line parsing against session log lines written by hand.

#[cfg(test)]
mod tests {
    use core::time::Duration;
    use ipc_layout::perf_line::{PerfLineError, PerfSummary, parse_line};

    const PREFIX: &str = "2026-11-13T12:35:25.951 INFO  thread=29780 arcane_eye_core::frame";
    const FIELDS: &str = "frames=600 mean_us=16667 p50_us=16650 p95_us=17100 p99_us=20480 \
                          max_us=33400 width=3840 height=2160";

    const SUMMARY: PerfSummary = PerfSummary {
        frames: 600,
        mean: Duration::from_micros(16_667),
        p50: Duration::from_micros(16_650),
        p95: Duration::from_micros(17_100),
        p99: Duration::from_micros(20_480),
        max: Duration::from_micros(33_400),
        width: 3840,
        height: 2160,
    };

    fn summary_line(fields: &str) -> String {
        format!("{PREFIX} msg=\"perf summary\" {fields}")
    }

    #[test]
    fn a_summary_line_yields_its_numbers() {
        assert_eq!(parse_line(&summary_line(FIELDS)), Ok(Some(SUMMARY)));
    }

    #[test]
    fn a_line_ending_after_the_last_field_is_ignored() {
        let line = summary_line(&format!("{FIELDS}\r\n"));
        assert_eq!(parse_line(&line), Ok(Some(SUMMARY)));
    }

    #[test]
    fn a_field_the_parser_does_not_know_is_skipped() {
        let line = summary_line(&format!("gpu_us=900 {FIELDS} note=\"a b\""));
        assert_eq!(parse_line(&line), Ok(Some(SUMMARY)));
    }

    #[test]
    fn a_line_of_another_message_yields_nothing() {
        let first_present =
            format!("{PREFIX} msg=\"first present\" width=3840 height=2160 format=24");
        let longer_message = format!("{PREFIX} msg=\"perf summary later\" {FIELDS}");
        let quoted_in_a_value =
            format!("{PREFIX} msg=\"saved\" path=\"x msg=\\\"perf summary\\\" {FIELDS}\"");
        assert_eq!(parse_line(&first_present), Ok(None));
        assert_eq!(parse_line(&longer_message), Ok(None));
        assert_eq!(parse_line(&quoted_in_a_value), Ok(None));
        assert_eq!(parse_line(""), Ok(None));
    }

    #[test]
    fn a_summary_line_with_a_missing_field_names_the_field() {
        let line = summary_line(&FIELDS.replace("p95_us=17100 ", ""));
        let error = parse_line(&line).expect_err("a missing field");
        assert_eq!(error, PerfLineError::MissingField { field: "p95_us" });
        assert_eq!(error.to_string(), "the perf summary line has no p95_us field");
    }

    #[test]
    fn a_summary_line_with_a_non_number_value_names_the_field_and_the_value() {
        let line = summary_line(&FIELDS.replace("p99_us=20480", "p99_us=20480[cut]"));
        let error = parse_line(&line).expect_err("a value that is not a number");
        assert!(
            matches!(&error, PerfLineError::NotANumber { field: "p99_us", value, .. } if value == "20480[cut]"),
            "{error:?}"
        );
        assert_eq!(
            error.to_string(),
            "the perf summary field p99_us holds \"20480[cut]\", which is not a whole number in range"
        );
    }

    #[test]
    fn a_value_past_the_field_range_is_not_a_number() {
        let line = summary_line(&FIELDS.replace("width=3840", "width=4294967296"));
        let error = parse_line(&line).expect_err("a width past u32");
        assert!(matches!(error, PerfLineError::NotANumber { field: "width", .. }), "{error:?}");
    }

    #[test]
    fn a_summary_line_with_a_repeated_field_is_rejected() {
        let line = summary_line(&format!("{FIELDS} frames=601"));
        let error = parse_line(&line).expect_err("a repeated field");
        assert_eq!(error, PerfLineError::RepeatedField { field: "frames" });
    }
}
