//! Camera line parsing against session log lines written by hand.

#[cfg(test)]
mod tests {
    use core::f32::consts::FRAC_PI_3;

    use ipc_layout::camera_line::{CameraEye, CameraLine, CameraLineError, parse_line};
    use ipc_layout::log_fields::is_written_quoted;

    const PREFIX: &str = "2026-11-13T12:35:25.951 INFO  thread=29780 arcane_eye_core::frame";
    const FIELDS: &str = "eye=right x=1000.625 y=-138.5 z=-1083.0625 head_x=1000.6417 \
                          head_y=-138.54124 head_z=-1083.1255 fx=-0.9248998 fy=0.3607648 \
                          fz=0.12003922 fov_y_rad=1.0471976 aspect=1.7777778 near=0.099999994 \
                          far=1000.0";

    const CAMERA: CameraLine = CameraLine {
        eye: CameraEye::Right,
        x: 1_000.625,
        y: -138.5,
        z: -1_083.062_5,
        head_x: 1_000.641_7,
        head_y: -138.541_24,
        head_z: -1_083.125_5,
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
    fn a_camera_line_yields_its_eye_and_numbers() {
        assert_eq!(parse_line(&camera_line(FIELDS)), Ok(Some(CAMERA)));
    }

    #[test]
    fn a_camera_line_displays_as_the_fields_the_parser_reads() {
        assert_eq!(CAMERA.to_string(), FIELDS);
    }

    #[test]
    fn the_two_camera_lines_of_one_window_each_yield_their_own_eye_and_numbers() {
        let left_fields = FIELDS.replace("eye=right x=1000.625", "eye=left x=1000.5625");
        let log = format!("{}\n{}\n", camera_line(&left_fields), camera_line(FIELDS));
        let parsed: Vec<_> = log.lines().map(parse_line).collect();
        let left = CameraLine { eye: CameraEye::Left, x: 1_000.562_5, ..CAMERA };
        assert_eq!(parsed, [Ok(Some(left)), Ok(Some(CAMERA))]);
    }

    #[test]
    fn each_eye_reads_and_displays_as_its_name() {
        for (eye, name) in
            [(CameraEye::Center, "center"), (CameraEye::Left, "left"), (CameraEye::Right, "right")]
        {
            let fields = FIELDS.replace("eye=right", &format!("eye={name}"));
            let camera = CameraLine { eye, ..CAMERA };
            assert_eq!(parse_line(&camera_line(&fields)), Ok(Some(camera)), "{name}");
            assert_eq!(camera.to_string(), fields, "{name}");
            assert_eq!(eye.as_str(), name);
            assert!(!is_written_quoted(name), "the session log writes {name} bare");
        }
    }

    #[test]
    fn a_camera_line_reads_and_displays_extreme_numbers_in_exponent_form() {
        let fields = "eye=center x=3.4028235e38 y=-3.4028235e38 z=1e-45 head_x=3.4028235e38 \
                      head_y=-3.4028235e38 head_z=1e-45 fx=-1e-45 fy=0.0 fz=1.0 \
                      fov_y_rad=1.0471976 aspect=1.7777778 near=1e-45 far=3.4028235e38";
        let smallest = f32::from_bits(1);
        let camera = CameraLine {
            eye: CameraEye::Center,
            x: f32::MAX,
            y: f32::MIN,
            z: smallest,
            head_x: f32::MAX,
            head_y: f32::MIN,
            head_z: smallest,
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
        let line = camera_line(&format!("frame=12 {FIELDS}\r\n"));
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
        let no_head = camera_line(&FIELDS.replace("head_y=-138.54124 ", ""));
        let head_error = parse_line(&no_head).expect_err("a missing head field");
        assert_eq!(head_error, CameraLineError::MissingField { field: "head_y" });
        let no_eye = camera_line(&FIELDS.replace("eye=right ", ""));
        let eye_error = parse_line(&no_eye).expect_err("a missing eye field");
        assert_eq!(eye_error, CameraLineError::MissingField { field: "eye" });
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
    fn a_camera_line_with_an_unknown_eye_names_the_value() {
        let line = camera_line(&FIELDS.replace("eye=right", "eye=Right"));
        let error = parse_line(&line).expect_err("an eye name in the wrong case");
        assert_eq!(error, CameraLineError::UnknownEye { value: "Right".to_owned() });
        assert_eq!(
            error.to_string(),
            "the camera field eye holds \"Right\", which is not center, left or right"
        );
    }

    #[test]
    fn a_camera_line_with_a_repeated_field_is_rejected() {
        let line = camera_line(&format!("{FIELDS} x=1"));
        let error = parse_line(&line).expect_err("a repeated field");
        assert_eq!(error, CameraLineError::RepeatedField { field: "x" });
        let eye_line = camera_line(&format!("{FIELDS} eye=left"));
        let eye_error = parse_line(&eye_line).expect_err("a repeated eye");
        assert_eq!(eye_error, CameraLineError::RepeatedField { field: "eye" });
    }
}
