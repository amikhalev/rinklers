#![warn(missing_docs)]

//! A program for managing a sprinkler system

#[macro_use]
extern crate log;
extern crate env_logger;
extern crate signal;
extern crate chrono;
extern crate num;
extern crate colored;

pub mod section;
pub mod section_runner;
pub mod program;
pub mod program_runner;
pub mod schedule;
pub mod schedule_runner;
pub mod wait_for;

pub use section::{Section, SectionRef, LogSection};
pub use section_runner::SectionRunner;
pub use program::Program;
pub use program_runner::ProgramRunner;
pub use schedule::{DateTimeBound, Schedule, every_day};

use std::time::Duration;
use std::env;
use std::sync::Arc;
use signal::trap::Trap;

fn init_log() {
    use log::{LogLevelFilter, LogLevel};
    use colored::Colorize;
    let mut log_builder = env_logger::LogBuilder::new();
    log_builder.filter(Some("rinklers"), LogLevelFilter::Debug);
    if let Ok(s) = env::var("RUST_LOG") {
        log_builder.parse(&s);
    }
    log_builder.format(|rec| {
        let level_str = match rec.level() {
            LogLevel::Trace => "[TRACE]".dimmed(),
            LogLevel::Debug => "[DEBUG]".white().bold(),
            LogLevel::Info => "[INFO]".cyan(),
            LogLevel::Warn => "[WARN]".yellow(),
            LogLevel::Error => "[ERROR]".red(),
        };
        format!("{:7} {:20} - {}",
                level_str,
                rec.location().module_path(),
                rec.args())
    });
    log_builder.init().unwrap();
}

fn main() {
    init_log();

    let mut sections: Vec<SectionRef>;
    let section_runner = SectionRunner::start_new();
    let program_runner = ProgramRunner::start_new(section_runner.clone());

    info!("initializing sections");
    sections = (0..6)
        .map(|i| format!("Section {}", i + 1))
        .map(|name| Arc::new(LogSection::new_noop(name)) as SectionRef)
        .collect();

    for section in sections.iter_mut() {
        section.set_state(false);
    }

    use chrono::NaiveTime;
    let schedule = Schedule::new(vec![NaiveTime::from_hms(14, 51, 0)],
                                 every_day(),
                                 DateTimeBound::None,
                                 DateTimeBound::None);
    let program = Program::new("Test Program",
                               vec![(sections[0].clone(), Duration::from_secs(2))],
                               schedule);
    let program = Arc::new(program);

    program_runner.add_program(program.clone());
    program.run_sync(&section_runner);

    Trap::trap(&[2, 15]).next(); // SIGINT, SIGKILL

    info!("received interrupt. stopping...");
    program_runner.stop();
    section_runner.stop();
}
