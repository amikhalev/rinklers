//! Contains [Program](struct.Program.html)

use std::sync::mpsc::Receiver;
use std::sync::Arc;
use std::iter::FromIterator;
use std::time::Duration;
use section::Section;
use section_runner::{SectionRunner, RunNotification};

type ProgItem = (Arc<Section>, Duration);

/// A list of sections and times, which can be run in sequence or be scheduled to run at certain
/// times
pub struct Program {
    name: String,
    sequence: Vec<ProgItem>,
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
    ///     [(section, std::time::Duration::from_seconds(10))]
    ///         .iter()
    ///         .clone()); // Must clone because this expects an iter of tuples, not references
    /// ```
    pub fn new<S: Into<String>, I: IntoIterator<Item = ProgItem>>(name: S, sequence: I) -> Program {
        Program {
            name: name.into(),
            sequence: Vec::from_iter(sequence),
        }
    }

    /// Gets the name of the `Program`
    pub fn name(&self) -> &String {
        &self.name
    }

    /// Runs the program, using `runner` to run each section for the period of time
    pub fn run(&self, runner: &SectionRunner) {
        debug!("running program {}", self.name);
        let mut iter = self.sequence.iter().peekable();
        let mut notifier: Option<Receiver<RunNotification>> = None;
        while let Some(item) = iter.next() {
            let run_notifier = runner.run_section(item.0.clone(), item.1);
            if iter.peek().is_none() {
                notifier = Some(run_notifier);
            }
        }
        if let Some(notifier) = notifier {
            loop {
                match notifier.recv().unwrap() {
                    RunNotification::Finish |
                    RunNotification::Interrupted => break,
                    _ => continue,
                }
            }
        }
        debug!("finished running program {}", self.name);
    }
}