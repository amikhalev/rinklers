//! Contains [Program](struct.Program.html)

use std::iter::FromIterator;
use std::sync::Mutex;
use super::*;

/// A single item in the sequence of execution of a program.
pub struct ProgItem {
    /// An `Arc` to the `Section` to run in the item in the sequence of the `Program`
    pub section: SectionRef,
    /// How long to run it for
    pub duration: Duration,
}

impl ProgItem {
    /// Creates a new `ProgItem` from a `SectionRef` and a `Duration`
    pub fn new(section: SectionRef, duration: Duration) -> Self {
        ProgItem {
            section: section,
            duration: duration,
        }
    }
}

/// A function that is called when a Program is updated
pub type UpdateFn = Box<Fn(&Program) + Send + Sync>;

/// A list of sections and times, which can be run in sequence or be scheduled to run at certain
/// times
pub struct Program {
    /// The name of the Program
    name: String,
    /// The sequence of items to run for the program
    sequence: Vec<ProgItem>,
    /// The `Schedule` that determines when the program will be run
    schedule: Schedule,
    on_schedule_update: Mutex<Option<UpdateFn>>,
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
                                                                  -> ProgramRef {
        Arc::new(Program {
            name: name.into(),
            sequence: Vec::from_iter(sequence),
            schedule: schedule,
            on_schedule_update: Mutex::new(None),
        })
    }

    /// Gets a reference to the name of this `Program`
    pub fn name(&self) -> &String {
        &self.name
    }

    /// Sets the name of this `Program` to `new_name`
    pub fn set_name(&mut self, new_name: String) {
        self.name = new_name;
    }

    /// Gets a reference to the sequence of this `Program`
    pub fn sequence(&self) -> &[ProgItem] {
        &self.sequence
    }

    /// Sets the sequence of this `Program` to `new_sequence`
    pub fn set_sequence<I: IntoIterator<Item = ProgItem>>(&mut self, new_sequence: I) {
        self.sequence = Vec::from_iter(new_sequence);
    }

    /// Gets a reference to the schedule of this `Program`
    pub fn schedule(&self) -> &Schedule {
        &self.schedule
    }

    /// Sets the schedule of this `Program` to `new_schedule`
    pub fn set_schedule(&mut self, new_schedule: Schedule) {
        self.schedule = new_schedule;
        self.on_schedule_update();
    }

    /// Runs the program, using `runner` to queue each section to be run for each specified period
    /// of time. Return the `RunNotifier`s for all of the sections that were queued
    pub fn queue_run(&self, runner: &SectionRunner) -> Vec<RunNotifier> {
        self.sequence
            .iter()
            .map(|item| runner.run_section(item.section.clone(), item.duration))
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

    /// Sets the function that is called when data on this `Program` is updated
    pub fn set_on_schedule_update(&self, on_schedule_update: Option<UpdateFn>) {
        *self.on_schedule_update.lock().unwrap() = on_schedule_update
    }

    fn on_schedule_update(&self) {
        let on_schedule_update = self.on_schedule_update.lock().unwrap();
        if let Some(ref update_fn) = *on_schedule_update {
            (update_fn)(self)
        }
    }
}

/// A reference to a `Program` (specifically `Arc`)
pub type ProgramRef = Arc<Program>;
