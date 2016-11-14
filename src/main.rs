#![warn(missing_docs)]

//! A program for managing a sprinkler system

#[macro_use]
extern crate log;
extern crate env_logger;
extern crate signal;

pub mod section;
pub mod section_runner;

use std::time::Duration;
use std::env;
use std::sync::Arc;
use signal::trap::Trap;
use log::LogLevelFilter;
use section::{Section, LogSection};
use section_runner::SectionRunner;

fn init_log() {
    let mut log_builder = env_logger::LogBuilder::new();
    log_builder.filter(Some("rinklers"), LogLevelFilter::Debug);
    if let Ok(s) = env::var("RUST_LOG") {
        log_builder.parse(&s);
    }
    log_builder.init().unwrap();
}

use std::iter::FromIterator;

type ProgItem = (Arc<Section>, Duration);

pub struct Program {
    name: String,
    sequence: Vec<ProgItem>,
}

impl Program {
    pub fn new<S: Into<String>, I: IntoIterator<Item=ProgItem>>(name: S, sequence: I) -> Program {
        Program { name: name.into(), sequence: Vec::from_iter(sequence) }
    }

    pub fn name(&self) -> &String {
        &self.name
    }

    pub fn run(&self, runner: &SectionRunner) {
        debug!("running program {}", self.name);
        for item in self.sequence.iter() {
            runner.run_section(item.0.clone(), item.1);
        }
    }
}

fn main() {
    init_log();

    let mut sections: Vec<Arc<Section>>;
    let section_runner = SectionRunner::start_new();

    info!("initializing sections");
    sections = (0..6)
        .map(|i| format!("Section {}", i + 1))
        .map(|name| Arc::new(LogSection::new(name)) as Arc<Section>)
        .collect();

    for section in sections.iter_mut() {
        section.set_state(false);
    }

    let program = Program::new("Test Program", [
                               (sections[0].clone(), Duration::from_secs(2)),
    ].iter().cloned());

    program.run(&section_runner);

    Trap::trap(&[2, 15]).next(); // SIGINT, SIGKILL

    info!("received interrupt. stopping...");
    section_runner.stop();
}

