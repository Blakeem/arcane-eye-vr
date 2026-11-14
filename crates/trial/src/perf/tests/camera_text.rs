//! The text of `camera.txt` for counted lines with no, one and two cameras.

use ipc_layout::camera_line::{CameraEye, CameraLine};

use super::super::{CountedLine, camera_text};
use super::{camera, eye_camera, summary};

/// A counted line with `cameras` before it.
fn counted(cameras: Vec<CameraLine>) -> CountedLine {
    CountedLine { summary: summary(600, 16_000, 20_000, 30_000), cameras }
}

#[test]
fn the_camera_text_holds_a_line_per_summary_line_or_one_none() {
    let expected = "\
eye=left x=1.5 y=2.0 z=3.0 head_x=7.0 head_y=2.0 head_z=3.0 fx=0.0 fy=0.0 fz=1.0 fov_y_rad=1.25 \
aspect=1.5 near=0.1 far=1000.0
none
eye=left x=-4.0 y=2.0 z=3.0 head_x=7.0 head_y=2.0 head_z=3.0 fx=0.0 fy=0.0 fz=1.0 fov_y_rad=1.25 \
aspect=1.5 near=0.1 far=1000.0
";
    let lines = [counted(vec![camera(1.5)]), counted(Vec::new()), counted(vec![camera(-4.0)])];
    assert_eq!(camera_text(&lines), expected);
    let empty = [counted(Vec::new()), counted(Vec::new())];
    assert_eq!(camera_text(&empty), "none\n", "a window with no camera");
    assert_eq!(camera_text(&[]), "none\n", "a window with no summary line");
}

#[test]
fn the_camera_text_holds_both_eyes_of_a_summary_line_on_its_line_in_log_order() {
    let expected = "\
eye=left x=1.5 y=2.0 z=3.0 head_x=7.0 head_y=2.0 head_z=3.0 fx=0.0 fy=0.0 fz=1.0 fov_y_rad=1.25 \
aspect=1.5 near=0.1 far=1000.0 | eye=right x=2.5 y=2.0 z=3.0 head_x=7.0 head_y=2.0 head_z=3.0 \
fx=0.0 fy=0.0 fz=1.0 fov_y_rad=1.25 aspect=1.5 near=0.1 far=1000.0
none
eye=left x=-4.0 y=2.0 z=3.0 head_x=7.0 head_y=2.0 head_z=3.0 fx=0.0 fy=0.0 fz=1.0 fov_y_rad=1.25 \
aspect=1.5 near=0.1 far=1000.0
";
    let pair = vec![camera(1.5), eye_camera(CameraEye::Right, 2.5)];
    let lines = [counted(pair), counted(Vec::new()), counted(vec![camera(-4.0)])];
    assert_eq!(camera_text(&lines), expected, "two cameras, no camera, then one camera");
}
