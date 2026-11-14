//! Camera line parsing against session log lines written by hand.

#[cfg(test)]
mod tests {
    use core::f32::consts::FRAC_PI_3;

    use ipc_layout::camera_line::{CameraLine, CameraLineError, parse_line};

    const PREFIX: &str = "2026-11-13T12:35:25.951 INFO  thread=29780 arcane_eye_core::frame";
    const FIELDS: &str = "x=1000.6417 y=-138.54124 z=-1083.1255 fx=-0.9248998 fy=0.3607648 \
                          fz=0.12003922 fov_y_rad=1.0471976 aspect=1.7777778 near=0.099999994 \
                          far=1000.0";

    const CAMERA: CameraLine = CameraLine {
        x: 1_000.641_7,
        y: -138.541_24,
        z: -1_083.125_5,
        fx: -0.924_899_8,
        fy: 0.360_764_8,
        fz: 0.120_039_22,
        fov_y_rad: FRAC_PI_3,
        aspect: 1.777_777_8,
        near: 0.099_999_994,
        far: 1_000.0,
    };

    fn camera_line(fields: &str) -> String {
        format!("{PREFIX} msg=camera {fields}")
    }

    #[test]
    fn a_camera_line_yields_its_numbers() {
        assert_eq!(parse_line(&camera_line(FIELDS)), Ok(Some(CAMERA)));
    }

    #[test]
    fn a_camera_line_displays_as_the_fields_the_parser_reads() {
        assert_eq!(CAMERA.to_string(), FIELDS);
    }

    #[test]
    fn a_camera_line_reads_and_displays_extreme_numbers_in_exponent_form() {
        let fields = "x=3.4028235e38 y=-3.4028235e38 z=1e-45 fx=-1e-45 fy=0.0 fz=1.0 \
                      fov_y_rad=1.0471976 aspect=1.7777778 near=1e-45 far=3.4028235e38";
        let smallest = f32::from_bits(1);
        let camera = CameraLine {
            x: f32::MAX,
            y: f32::MIN,
            z: smallest,
            fx: -smallest,
            fy: 0.0,
            fz: 1.0,
            near: smallest,
            far: f32::MAX,
            ..CAMERA
        };
        assert_eq!(parse_line(&camera_line(fields)), Ok(Some(camera)));
        assert_eq!(camera.to_string(), fields);
    }

    #[test]
    fn a_line_ending_and_an_unknown_field_are_ignored() {
        let line = camera_line(&format!("eye=left {FIELDS}\r\n"));
        assert_eq!(parse_line(&line), Ok(Some(CAMERA)));
    }

    #[test]
    fn a_line_of_another_message_yields_nothing() {
        let perf = format!("{PREFIX} msg=\"perf summary\" frames=600");
        let longer_message = format!("{PREFIX} msg=cameras {FIELDS}");
        let quoted_camera_message = format!("{PREFIX} msg=\"camera\" {FIELDS}");
        assert_eq!(parse_line(&perf), Ok(None));
        assert_eq!(parse_line(&longer_message), Ok(None));
        assert_eq!(parse_line(&quoted_camera_message), Ok(None));
        assert_eq!(parse_line(""), Ok(None));
    }

    #[test]
    fn a_camera_line_with_a_missing_field_names_the_field() {
        let line = camera_line(&FIELDS.replace("fov_y_rad=1.0471976 ", ""));
        let error = parse_line(&line).expect_err("a missing field");
        assert_eq!(error, CameraLineError::MissingField { field: "fov_y_rad" });
        assert_eq!(error.to_string(), "the camera line has no fov_y_rad field");
    }

    #[test]
    fn a_camera_line_with_a_non_number_value_names_the_field_and_the_value() {
        let line = camera_line(&FIELDS.replace("near=0.099999994", "near=0.1m"));
        let error = parse_line(&line).expect_err("a value that is not a number");
        assert!(
            matches!(&error, CameraLineError::NotANumber { field: "near", value, .. } if value == "0.1m"),
            "{error:?}"
        );
        assert_eq!(
            error.to_string(),
            "the camera field near holds \"0.1m\", which is not a number"
        );
    }

    #[test]
    fn a_camera_line_with_a_repeated_field_is_rejected() {
        let line = camera_line(&format!("{FIELDS} x=1"));
        let error = parse_line(&line).expect_err("a repeated field");
        assert_eq!(error, CameraLineError::RepeatedField { field: "x" });
    }
}
