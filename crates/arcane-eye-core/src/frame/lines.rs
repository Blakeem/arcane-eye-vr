//! The lines the frame handler logs once per perf window before the perf summary line: the camera
//! lines, the UI depth's `ui depth` and `cursor depth` lines and, in the launch mode `vr`, the
//! `vr view` line. The perf window limits each line to one per `SUMMARY_FRAMES` frames, so none
//! needs a rate-limit macro. Each line carries the frame handler's target, as the perf summary line
//! does.

use ipc_layout::camera_line::{self, CameraEye};
use stereo_core::eye::Eye;

use super::TARGET;
use crate::ui_layer::ui_depth::{CursorReport, DepthReport};
use crate::view_camera::{FrameCamera, Viewpoint};
use crate::vr_view::VrReport;

/// Logs one camera line. A reader pairs it with the perf summary line that follows it.
pub(super) fn log_camera(frame: &FrameCamera) {
    let camera = &frame.camera;
    let [x, y, z] = camera.position;
    let [head_x, head_y, head_z] = frame.head();
    let [fx, fy, fz] = camera.forward;
    // Debug writes an extreme f32 in exponent form, which keeps every field inside one log slot.
    tracing::info!(
        target: TARGET,
        eye = camera_eye(frame.viewpoint).as_str(),
        x = ?x,
        y = ?y,
        z = ?z,
        head_x = ?head_x,
        head_y = ?head_y,
        head_z = ?head_z,
        fx = ?fx,
        fy = ?fy,
        fz = ?fz,
        fov_y_rad = ?camera.fov_y_rad,
        aspect = ?camera.aspect,
        near = ?camera.near,
        far = ?camera.far,
        "{}",
        camera_line::MESSAGE
    );
}

/// Logs the UI depth's lines. The `ui depth` line holds the nearest distance of
/// [`DepthReport::nearest_m`], the columns per half of the UI disparity in use, the eye scale in
/// use, the depth offset in use as a share and where it comes from, `screen`, `setting` or `none`
/// for no depth offset. The `cursor depth` line holds what the newest pointer read's result is
/// over, `ui`, `scene` or `none` for no result, and the cursor's moves in use in the left half and
/// the right half.
pub(super) fn log_depth(depth: DepthReport, cursor: CursorReport) {
    tracing::info!(
        target: TARGET,
        nearest_m = ?depth.nearest_m(),
        ui_columns = depth.columns,
        eye_scale = ?depth.eye_scale.share(),
        offset = ?depth.offset.share(),
        offset_from = depth.offset.origin().as_str(),
        "ui depth"
    );
    tracing::info!(
        target: TARGET,
        over = cursor.over.as_str(),
        left_columns = cursor.moves.left,
        right_columns = cursor.moves.right,
        "cursor depth"
    );
}

/// Logs the `vr view` line: the view kind decided at the newest Present call, `world` or `screen`,
/// where the head pose in use at that call came from, its sequence number, its turn to the right
/// and upward in degrees, the eye distance in meters and the dev flag `vr.horizon`.
pub(super) fn log_vr(report: VrReport) {
    tracing::info!(
        target: TARGET,
        view = report.view.as_str(),
        pose = report.source.as_str(),
        head_sequence = report.head_sequence,
        yaw_deg = ?report.yaw_deg,
        pitch_deg = ?report.pitch_deg,
        eye_m = ?report.eye_m,
        horizon = report.horizon.as_str(),
        "vr view"
    );
}

/// The camera line's name for the position a view rendered from.
const fn camera_eye(viewpoint: Viewpoint) -> CameraEye {
    match viewpoint {
        Viewpoint::Head => CameraEye::Center,
        Viewpoint::Eye { eye: Eye::Left, .. } => CameraEye::Left,
        Viewpoint::Eye { eye: Eye::Right, .. } => CameraEye::Right,
    }
}
