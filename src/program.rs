//! Contains [Program](struct.Program.html)

use std::sync::Arc;
use std::iter::FromIterator;
use std::time::Duration;
use section::Section;
use section_runner::{SectionRunner, RunNotification, RunNotifier};
use schedule::Schedule;

/// A single item in the sequence of execution of a program. Contains an `Arc` to a section to run
/// and a `Duration` to run it for.
pub type ProgItem = (Arc<Section>, Duration);

/// A list of sections and times, which can be run in sequence or be scheduled to run at certain
/// times
pub struct Program {
    /// The name of the Program
    pub name: String,
    /// The sequence of items to run for the program
    pub sequence: Vec<ProgItem>,
    /// The `Schedule` that determines when the program will be run
    pub schedule: Schedule,
}

impl Program {
    /// Creates a new `Program`, with the specified name and run sequence
    ///
    /// The sequence is a list of pairs of Sections and Durations. The sections will be run for
    /// with their matching durations in the sequence they are declared in.
    ///
    /// # Examples
    /// ```
    /// use rinklers::*;
    /// let section = LogSection::new("Test Section");
    /// let program = Program::new("Test Program",
    ///     vec![(section, std::time::Duration::from_seconds(10))],
    ///     Schedule::default());
    /// ```
    pub fn new<S: Into<String>, I: IntoIterator<Item = ProgItem>>(name: S,
                                                                  sequence: I,
                                                                  schedule: Schedule)
                                                                  -> Program {
        Program {
            name: name.into(),
            sequence: Vec::from_iter(sequence),
            schedule: schedule,
        }
    }

    /// Runs the program, using `runner` to queue each section to be run for each specified period
    /// of time. Return the `RunNotifier`s for all of the sections that were queued
    pub fn queue_run<'a, 'b>(&'a self, runner: &'b SectionRunner) -> Vec<RunNotifier> {
        self.sequence
            .iter()
            .map(|item| runner.run_section(item.0.clone(), item.1))
            .collect()
    }

    /// Runs this program synchronously, blocking until the whole program finishes before returning.
    pub fn run_sync(&self, runner: &SectionRunner) {
        debug!("running program {}", self.name);
        let notifiers = self.queue_run(runner);
        if let Some(notifier) = notifiers.last() {
            for notification in notifier {
                match notification {
                    RunNotification::Finish |
                    RunNotification::Interrupted => break,
                    _ => continue,
                }
            }
        }
        debug!("finished running program {}", self.name);
    }
}

/// A reference to a `Program` (specifically `Arc`)
pub type ProgramRef = Arc<Program>;
