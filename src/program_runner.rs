//! Contains [ProgramRunner](struct.ProgramRunner.html)
use schedule_runner::{Executor, ScheduleRunner};
use section_runner::SectionRunner;
use program::ProgramRef;

struct ProgramExecutor {
    section_runner: SectionRunner,
}

impl ProgramExecutor {
    fn new(section_runner: SectionRunner) -> Self {
        ProgramExecutor { section_runner: section_runner }
    }
}

impl Executor for ProgramExecutor {
    type Data = ProgramRef;

    fn execute(&self, data: &Self::Data) {
        let data = data.clone();
        debug!("running program {} at scheduled time", data.name);
        data.queue_run(&self.section_runner);
    }

    fn data_string(data: &Self::Data) -> String {
        format!("Program {{ name: {} }}", data.name)
    }
}

/// Runs programs based on their schedules
pub struct ProgramRunner {
    // programs: Vec<ProgramRef>,
    sched_runner: ScheduleRunner<ProgramExecutor>,
}

impl ProgramRunner {
    /// Starts a new `ProgramRunner` thread and returns the object used to control it
    pub fn start_new(section_runner: SectionRunner) -> Self {
        let executor = ProgramExecutor::new(section_runner);
        ProgramRunner {
            // programs: Vec::new(),
            sched_runner: ScheduleRunner::start_new(executor),
        }
    }

    /// Schedules a program
    pub fn add_program(&self, program: ProgramRef) {
        self.sched_runner.schedule(program.schedule.clone(), program);
    }

    /// Stops the `ProgramRunner`
    pub fn stop(self) {
        self.sched_runner.stop();
    }
}
