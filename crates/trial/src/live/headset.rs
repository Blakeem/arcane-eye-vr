//! The launch app and SteamVR's headset window of a real trial. The launch app runs from the folder
//! that holds this program, and the launch crate copies the headset window from the screen.

use std::env;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use ipc_layout::root_folder::HOME_VARIABLE;
use launch::game_dir;
use launch::stop;
use launch::window_copy::{self, WindowPicture};
use tracing::info;

use super::wait;
use crate::headset::HeadsetError;
use crate::pixels::{Depth, RgbImage};
use crate::steps::headset::Headset;
use crate::steps::{Step, StepError};

/// The launch app's program file, which `tools/deploy.ps1` stages beside this program.
const LAUNCH_APP: &str = "arcane-eye-vr.exe";

/// The launch app's arguments that run its screen mode beside a game that already runs.
const SCREEN_MODE: [&str; 2] = ["--mode", "screen"];

/// The program of SteamVR's compositor, which owns the headset window.
const COMPOSITOR: &str = "vrcompositor";

/// The title of the window that shows what the headset shows, both eyes side by side.
const HEADSET_WINDOW: &str = "Headset Window";

/// The time between two reads of whether the launch app exited.
const EXIT_POLL: Duration = Duration::from_millis(250);

/// The launch app, found by the check step and started by the headset-screen step.
pub(crate) struct LiveHeadset {
    /// The launch app's path once the check found it.
    app: Option<PathBuf>,
    /// The launch app's process once it started. It keeps the process id bound to the launch app.
    child: Option<Child>,
}

impl LiveHeadset {
    pub(crate) const fn new() -> Self {
        Self { app: None, child: None }
    }

    fn child(&mut self) -> Result<&mut Child, StepError> {
        self.child.as_mut().ok_or(StepError::Needs(Step::HeadsetScreen))
    }
}

impl Headset for LiveHeadset {
    fn check(&mut self) -> Result<(), StepError> {
        let program = match env::current_exe() {
            Ok(program) => program,
            Err(source) => return Err(HeadsetError::Program(source).into()),
        };
        let path = program.with_file_name(LAUNCH_APP);
        match game_dir::is_file(&path) {
            Ok(true) => {
                self.app = Some(path);
                Ok(())
            }
            Ok(false) => Err(HeadsetError::AppMissing { path }.into()),
            Err(source) => Err(HeadsetError::AppCheck { path, source }.into()),
        }
    }

    fn start(&mut self, home: &Path, output: &Path) -> Result<(), StepError> {
        let app = self.app.clone().ok_or(StepError::Needs(Step::Check))?;
        let created = File::create(output).and_then(|out| Ok((out.try_clone()?, out)));
        let (errors, out) = match created {
            Ok(files) => files,
            Err(source) => {
                return Err(HeadsetError::Output { path: output.to_path_buf(), source }.into());
            }
        };
        let mut command = Command::new(&app);
        command.args(SCREEN_MODE).env(HOME_VARIABLE, home);
        command.stdin(Stdio::null()).stdout(out).stderr(errors);
        let child = match command.spawn() {
            Ok(child) => child,
            Err(source) => return Err(HeadsetError::Start { path: app, source }.into()),
        };
        let (pid, app, home) = (child.id(), app.display(), home.display());
        info!(pid, app = %app, home = %home, "started the launch app's screen mode");
        self.child = Some(child);
        Ok(())
    }

    fn started(&self) -> bool {
        self.child.is_some()
    }

    fn copy_window(&mut self) -> Result<Option<RgbImage>, StepError> {
        let copy = window_copy::copy(COMPOSITOR, HEADSET_WINDOW)?;
        Ok(copy.map(|WindowPicture { width, height, rgb }| RgbImage {
            width,
            height,
            depth: Depth::Eight,
            data: rgb,
        }))
    }

    fn wait_exit(&mut self, timeout: Duration) -> Result<Option<ExitStatus>, StepError> {
        let child = self.child()?;
        let started = Instant::now();
        loop {
            match child.try_wait() {
                Ok(Some(status)) => return Ok(Some(status)),
                Ok(None) if started.elapsed() >= timeout => return Ok(None),
                Ok(None) => wait(EXIT_POLL),
                Err(source) => return Err(HeadsetError::AppState(source).into()),
            }
        }
    }

    fn end(&mut self) -> Result<(), StepError> {
        let pid = self.child()?.id();
        Ok(stop::end(pid)?)
    }
}

#[cfg(test)]
mod tests {
    use std::env;
    use std::fs;
    use std::io::{self, Read as _};
    use std::process::{Command, Stdio};
    use std::time::Duration;

    use super::{LAUNCH_APP, LiveHeadset};
    use crate::headset::HeadsetError;
    use crate::steps::StepError;
    use crate::steps::headset::Headset;
    use crate::test_dir;

    /// The libtest name of the test that a child process runs as a launch app that does not exit.
    const RUNNING_APP_TEST: &str = "live::headset::tests::stand_in_app_runs_until_its_input_ends";

    #[test]
    #[ignore = "runs only as the child process of another test"]
    fn stand_in_app_runs_until_its_input_ends() {
        let mut input = Vec::new();
        io::stdin().read_to_end(&mut input).expect("read the input until it ends");
    }

    #[test]
    fn a_launch_app_that_still_runs_at_the_timeout_waits_as_running_and_its_end_stops_it() {
        let program = env::current_exe().expect("the test binary path");
        let mut child = Command::new(program)
            .args(["--exact", RUNNING_APP_TEST, "--ignored"])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("start the stand-in app");
        // Held open, so the stand-in still runs until its end.
        let _input = child.stdin.take();
        let mut headset = LiveHeadset::new();
        headset.child = Some(child);
        let waited = headset.wait_exit(Duration::from_millis(300)).expect("read the state");
        assert_eq!(waited, None, "the stand-in exited with its input open");
        headset.end().expect("end the stand-in");
        let ended = headset.wait_exit(Duration::ZERO).expect("read the exit");
        assert!(ended.is_some_and(|status| !status.success()), "{ended:?}");
    }

    #[test]
    fn the_check_fails_with_the_path_beside_this_program_when_the_launch_app_is_missing() {
        let mut headset = LiveHeadset::new();
        let error = headset.check().expect_err("no launch app beside the test binary");
        let StepError::Headset(HeadsetError::AppMissing { path }) = error else {
            panic!("not a missing launch app: {error:?}");
        };
        let program = env::current_exe().expect("the test binary path");
        assert_eq!(path, program.with_file_name(LAUNCH_APP));
        assert!(!headset.started());
    }

    #[test]
    fn a_started_app_writes_its_output_to_the_file_and_its_exit_is_read() {
        let dir = test_dir("live-headset-start");
        let output = dir.path().join("screen-app.txt");
        // The test binary stands in for the launch app. libtest rejects `--mode` and exits.
        let mut headset = LiveHeadset::new();
        headset.app = Some(env::current_exe().expect("the test binary path"));
        headset.start(dir.path(), &output).expect("start the stand-in app");
        assert!(headset.started());
        let status = headset.wait_exit(Duration::from_secs(30)).expect("read the exit");
        let status = status.expect("the stand-in exited within 30 s");
        assert!(!status.success(), "libtest took --mode screen: {status}");
        let written = fs::read_to_string(&output).expect("read the output file");
        assert!(written.contains("mode"), "{written}");
        headset.end().expect("an end of an exited process is not an error");
    }

    #[test]
    fn a_start_before_the_check_found_the_app_needs_the_check_step() {
        let dir = test_dir("live-headset-unchecked");
        let mut headset = LiveHeadset::new();
        let output = dir.path().join("screen-app.txt");
        let error = headset.start(dir.path(), &output).expect_err("no app path");
        assert!(matches!(error, StepError::Needs(crate::steps::Step::Check)), "{error:?}");
        assert!(!headset.started());
    }
}
