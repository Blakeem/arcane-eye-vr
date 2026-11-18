//! The trial's report: the trial desktop, how each step that ran ended, the text of `result.txt`
//! and the exit code.

use std::process::ExitCode;
use std::time::Duration;

use tracing::{info, warn};

use crate::steps::{Closed, Step, StepError, TRIAL_FAILED, cause};

/// How one step ended.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Outcome {
    Passed,
    Forced { waited: Duration },
    Failed { cause: String },
}

/// The trial desktop the game ran on, the steps that ran and how each ended.
#[derive(Debug)]
pub(crate) struct Report {
    /// `None` for the desktop the trial started on.
    desktop: Option<String>,
    steps: Vec<(Step, Outcome)>,
}

impl Report {
    /// An empty report of a trial whose game runs on `desktop`.
    pub(crate) fn on_desktop(desktop: Option<&str>) -> Self {
        Self { desktop: desktop.map(str::to_owned), steps: Vec::new() }
    }

    /// Records the result of `step` and returns whether it passed.
    pub(crate) fn record(&mut self, step: Step, result: Result<(), StepError>) -> bool {
        let outcome = match result {
            Ok(()) => Outcome::Passed,
            Err(error) => Outcome::Failed { cause: cause(&error) },
        };
        self.push(step, outcome)
    }

    /// Records the result of `step`, which closes the game, and returns whether it passed. A forced
    /// close passes and is named in `result.txt`.
    pub(crate) fn record_close(&mut self, step: Step, result: Result<Closed, StepError>) -> bool {
        let outcome = match result {
            Ok(Closed::Exited) => Outcome::Passed,
            Ok(Closed::Forced { waited }) => Outcome::Forced { waited },
            Err(error) => Outcome::Failed { cause: cause(&error) },
        };
        self.push(step, outcome)
    }

    fn push(&mut self, step: Step, outcome: Outcome) -> bool {
        let passed = !matches!(outcome, Outcome::Failed { .. });
        match &outcome {
            Outcome::Failed { cause } => warn!(step = step.as_str(), cause = %cause, "step failed"),
            Outcome::Passed | Outcome::Forced { .. } => info!(step = step.as_str(), "step passed"),
        }
        self.steps.push((step, outcome));
        passed
    }

    fn passed(&self) -> bool {
        self.steps.iter().all(|(_, outcome)| !matches!(outcome, Outcome::Failed { .. }))
    }

    /// 0 when every step passed and 1 when a step failed.
    pub(crate) fn exit_code(&self) -> ExitCode {
        if self.passed() { ExitCode::SUCCESS } else { ExitCode::from(TRIAL_FAILED) }
    }

    /// The text of `result.txt`: the trial desktop when the game ran on one, one line per step that
    /// ran, then the outcome.
    pub(crate) fn text(&self) -> String {
        let desktop = self.desktop.iter().map(|name| format!("desktop: {name}\n"));
        let steps = self.steps.iter().map(|(step, outcome)| step_line(*step, outcome));
        let mut lines: Vec<String> = desktop.chain(steps).collect();
        let outcome = if self.passed() { "passed" } else { "failed" };
        lines.push(format!("outcome: {outcome}\n"));
        lines.concat()
    }
}

/// The line of `result.txt` for `step`, which ended with `outcome`.
fn step_line(step: Step, outcome: &Outcome) -> String {
    match outcome {
        Outcome::Passed => format!("{}: passed\n", step.as_str()),
        Outcome::Forced { waited } => format!(
            "{}: passed, forced: the game still ran {waited:?} after the close request, so it was \
             ended\n",
            step.as_str()
        ),
        Outcome::Failed { cause } => format!("{}: failed: {cause}\n", step.as_str()),
    }
}
