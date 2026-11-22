//! The choice of the launcher window among the top-level windows of the desktop.

use std::path::PathBuf;

use super::{Candidate, PROGRAM, Showing, choose};

/// A candidate numbered `window` of the program file `program` with a client area of `area`.
fn window(window: u32, showing: Showing, program: &str, area: (i32, i32)) -> Candidate<u32> {
    let program = Some(PathBuf::from(format!(r"C:\Games\Baldurs Gate 3\Launcher\{program}")));
    Candidate { window, showing, program, area }
}

/// The number of the chosen window among `candidates`.
fn chosen(candidates: &[Candidate<u32>]) -> Option<u32> {
    choose(candidates, PROGRAM).map(|candidate| candidate.window)
}

#[test]
fn the_launcher_window_is_the_one_with_the_largest_client_area() {
    let candidates = [
        window(1, Showing::Visible, PROGRAM, (300, 200)),
        window(2, Showing::Visible, PROGRAM, (1280, 720)),
        window(3, Showing::Visible, PROGRAM, (1000, 1000)),
    ];
    assert_eq!(chosen(&candidates), Some(3));
}

#[test]
fn of_two_equal_client_areas_the_first_window_is_chosen() {
    let candidates = [
        window(4, Showing::Visible, PROGRAM, (640, 480)),
        window(5, Showing::Visible, PROGRAM, (480, 640)),
    ];
    assert_eq!(chosen(&candidates), Some(4));
}

#[test]
fn a_minimized_or_hidden_window_is_never_chosen() {
    let candidates = [
        window(1, Showing::Minimized, PROGRAM, (1920, 1080)),
        window(2, Showing::Hidden, PROGRAM, (1920, 1080)),
        window(3, Showing::Visible, PROGRAM, (200, 100)),
    ];
    assert_eq!(chosen(&candidates), Some(3));
    let none_shown = candidates.get(..2).expect("two candidates");
    assert_eq!(chosen(none_shown), None);
}

#[test]
fn a_window_of_another_program_or_of_an_unread_program_is_never_chosen() {
    let unread = Candidate { program: None, ..window(2, Showing::Visible, PROGRAM, (1920, 1080)) };
    let candidates = [
        window(1, Showing::Visible, "steamwebhelper.exe", (1920, 1080)),
        unread,
        window(3, Showing::Visible, "LariLauncher.exe.bak", (1920, 1080)),
    ];
    assert_eq!(chosen(&candidates), None);
    let launcher = window(4, Showing::Visible, "LARILAUNCHER.EXE", (800, 600));
    let with_launcher = [candidates.as_slice(), &[launcher]].concat();
    assert_eq!(chosen(&with_launcher), Some(4), "the name matches in any case");
}

#[test]
fn a_window_with_an_empty_client_area_is_never_chosen() {
    let candidates = [
        window(1, Showing::Visible, PROGRAM, (0, 0)),
        window(2, Showing::Visible, PROGRAM, (1920, 0)),
        window(3, Showing::Visible, PROGRAM, (0, 1080)),
    ];
    assert_eq!(chosen(&candidates), None);
    let launcher = window(4, Showing::Visible, PROGRAM, (1, 1));
    let with_launcher = [candidates.as_slice(), &[launcher]].concat();
    assert_eq!(chosen(&with_launcher), Some(4));
}

#[test]
fn no_candidate_gives_no_launcher_window() {
    assert_eq!(chosen(&[]), None);
}
