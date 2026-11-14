//! The `PerCamera` decode against the block measured on the title screen of build 4.1.1.7631656.

use core::f32::consts::FRAC_PI_3;

use super::{Camera, PerCamera};

/// The main view's `PerCamera` block 0, read at the camera-built site on the title screen.
const MEASURED: PerCamera = PerCamera {
    view: [
        0.131_419_54,
        0.356_776_65,
        -0.924_899_8,
        0.0,
        0.007_081_553_3,
        0.932_629_94,
        0.360_764_8,
        0.0,
        0.991_301_8,
        -0.053_961_32,
        0.120_039_22,
        0.0,
        943.181_46,
        -286.244_78,
        1_105.491_7,
        1.0,
    ],
    projection: [
        0.974_278_7,
        0.0,
        0.0,
        0.0,
        0.0,
        1.732_051,
        0.0,
        0.0,
        0.0,
        0.0,
        -0.000_100_009_99,
        1.0,
        0.0,
        0.0,
        0.100_009_99,
        0.0,
    ],
    near: 0.099_999_994,
    far: 1000.0,
    position: [1_000.641_7, -138.541_24, -1_083.125_5],
};

/// The tolerance of a decoded number against its expected value.
const TOLERANCE: f32 = 1.0e-5;

fn assert_near(actual: f32, expected: f32, what: &str) {
    assert!((actual - expected).abs() <= TOLERANCE, "{what}: {actual} is not {expected}");
}

fn decoded(block: &PerCamera) -> Camera {
    block.camera().expect("a perspective block")
}

#[test]
fn the_measured_block_decodes_to_its_position_axis_and_lens() {
    let camera = decoded(&MEASURED);
    let position = [1_000.641_7, -138.541_24, -1_083.125_5];
    let forward = [-0.924_899_8, 0.360_764_8, 0.120_039_22];
    for (axis, (actual, expected)) in camera.position.iter().zip(position).enumerate() {
        assert_near(*actual, expected, &format!("position {axis}"));
    }
    for (axis, (actual, expected)) in camera.forward.iter().zip(forward).enumerate() {
        assert_near(*actual, expected, &format!("forward {axis}"));
    }
    // The controller of the measured view holds these lens values.
    assert_near(camera.fov_y_rad, FRAC_PI_3, "vertical field of view");
    assert_near(camera.aspect, 1.777_777_8, "aspect");
    assert_near(camera.near, 0.1, "near");
    assert_near(camera.far, 1000.0, "far");
}

#[test]
fn the_measured_view_matrix_moves_the_measured_position_to_the_origin() {
    let [x, y, z] = MEASURED.position;
    let point = [x, y, z, 1.0];
    for row in 0..3 {
        let moved: f32 = (0..4).map(|column| MEASURED.view[column * 4 + row] * point[column]).sum();
        assert!(moved.abs() <= 0.01, "camera space axis {row} holds {moved}");
    }
}

#[test]
fn a_block_that_is_not_a_finite_perspective_projection_has_no_camera() {
    let with = |element: usize, value: f32| {
        let mut block = MEASURED;
        block.projection[element] = value;
        block
    };
    let cases = [
        ("element 11 is 0, an orthographic projection", with(11, 0.0)),
        ("element 11 is -1", with(11, -1.0)),
        ("element 15 is 1", with(15, 1.0)),
        ("element 0 is 0", with(0, 0.0)),
        ("element 0 is infinite", with(0, f32::INFINITY)),
        ("element 0 is NaN", with(0, f32::NAN)),
        ("element 5 is 0", with(5, 0.0)),
        ("element 5 is infinite", with(5, f32::NEG_INFINITY)),
        ("element 5 is NaN", with(5, f32::NAN)),
    ];
    for (case, block) in cases {
        assert_eq!(block.camera(), None, "{case}");
    }
    let zero = PerCamera {
        view: [0.0; 16],
        projection: [0.0; 16],
        near: 0.0,
        far: 0.0,
        position: [0.0; 3],
    };
    assert_eq!(zero.camera(), None, "the special view's all-zero block");
}
