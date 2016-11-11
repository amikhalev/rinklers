#[macro_use]
extern crate log;
extern crate env_logger;
extern crate signal;

mod section;
mod section_runner;

use std::time::Duration;
use std::env;
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

fn main() {
    init_log();

    info!("initializing sections");
    let mut sections: Vec<Box<Section>>;
    sections = (0..6)
        .map(|i| format!("Section {}", i + 1))
        .map(|name| Box::new(LogSection::new(name)) as Box<Section>)
        .collect();

    let section_runner = SectionRunner::start_new();
    for section in sections.iter_mut() {
        section.set_state(false);
        section_runner.queue_section_run(&**section, Duration::from_secs(1));
    }

    Trap::trap(&[2, 15]).next();

    info!("received interrupt. stopping...");
    section_runner.stop();
}

