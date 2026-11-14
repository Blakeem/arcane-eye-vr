//! The trial's report: how each step that ran ended, the text of `result.txt` and the exit code.

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

/// The steps that ran and how each ended.
#[derive(Debug, Default)]
pub(crate) struct Report {
    steps: Vec<(Step, Outcome)>,
}

impl Report {
    /// Records the result of `step` and returns whether it passed.
    pub(crate) fn record(&mut self, step: Step, result: Result<(), StepError>) -> bool {
        let outcome = match result {
            Ok(()) => Outcome::Passed,
            Err(error) => Outcome::Failed { cause: cause(&error) },
        };
        self.push(step, outcome)
    }

    pub(crate) fn record_close(&mut self, result: Result<Closed, StepError>) {
        let outcome = match result {
            Ok(Closed::Exited) => Outcome::Passed,
            Ok(Closed::Forced { waited }) => Outcome::Forced { waited },
            Err(error) => Outcome::Failed { cause: cause(&error) },
        };
        self.push(Step::Close, outcome);
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

    /// The text of `result.txt`: one line per step that ran, then the outcome.
    pub(crate) fn text(&self) -> String {
        let mut lines: Vec<String> = self
            .steps
            .iter()
            .map(|(step, outcome)| match outcome {
                Outcome::Passed => format!("{}: passed\n", step.as_str()),
                Outcome::Forced { waited } => format!(
                    "{}: passed, forced: the game still ran {waited:?} after the close request, so \
                     it was ended\n",
                    step.as_str()
                ),
                Outcome::Failed { cause } => format!("{}: failed: {cause}\n", step.as_str()),
            })
            .collect();
        let outcome = if self.passed() { "passed" } else { "failed" };
        lines.push(format!("outcome: {outcome}\n"));
        lines.concat()
    }
}
