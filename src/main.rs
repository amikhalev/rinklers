#![feature(proc_macro)]
#![warn(missing_docs)]

//! A program for managing a sprinkler system

#[macro_use]
extern crate log;
#[macro_use]
extern crate serde_derive;
#[macro_use]
extern crate quick_error;

extern crate env_logger;
extern crate signal;
extern crate chrono;
extern crate time;
extern crate num;
extern crate colored;
extern crate serde;
extern crate serde_json;

pub mod section;
pub mod section_runner;
pub mod program;
pub mod program_runner;
pub mod schedule;
pub mod schedule_runner;
pub mod util;

pub use section::{Section, SectionRef, LogSection};
pub use section_runner::SectionRunner;
pub use program::{ProgItem, Program, ProgramRef};
pub use program_runner::ProgramRunner;
pub use schedule::{DateTimeBound, Schedule, every_day};

use std::time::Duration;
use std::env;
use std::sync::Arc;
use signal::trap::Trap;

#[derive(Serialize, Deserialize, Debug)]
struct SectionConfig {
    name: String,
}

impl SectionConfig {
    fn to_section(&self) -> Arc<Section> {
        // TODO: Check some environment variable and change the type of section created
        Arc::new(LogSection::new_noop(&self.name as &str)) as SectionRef
    }
}

quick_error!{
    #[derive(Debug)]
    enum ConfigError {
        SectionOutOfRange(sec_idx: usize) {
            description("Section index was out of range")
            display("Section index {} is out of range", sec_idx)
        }
    }
}

type ConfigResult<T> = Result<T, ConfigError>;

#[derive(Serialize, Deserialize, Debug)]
struct ProgItemConfig {
    section: usize,
    #[serde(deserialize_with="util::deserialize_duration")]
    duration: Duration,
}

impl ProgItemConfig {
    fn to_prog_item(&self, sections: &[SectionRef]) -> ConfigResult<ProgItem> {
        if self.section >= sections.len() {
            Err(ConfigError::SectionOutOfRange(self.section))
        } else {
            Ok(ProgItem::new(sections[self.section].clone(), self.duration))
        }
    }
}

#[derive(Serialize, Deserialize, Debug)]
struct ProgramConfig {
    name: String,
    sequence: Box<[ProgItemConfig]>,
    schedule: Schedule,
}

impl ProgramConfig {
    fn to_program(&self, sections: &[SectionRef]) -> ConfigResult<ProgramRef> {
        let sequence: ConfigResult<Vec<ProgItem>> = self.sequence
            .iter()
            .map(|prog_item| prog_item.to_prog_item(sections))
            .collect();
        let sequence = try!(sequence);
        Ok(Program::new(self.name.clone(), sequence, self.schedule.clone()))
    }
}

#[derive(Serialize, Deserialize, Debug)]
struct Config {
    sections: Vec<SectionConfig>,
    programs: Vec<ProgramConfig>,
}

impl Config {
    fn to_sections(&self) -> Vec<SectionRef> {
        self.sections
            .iter()
            .map(|sec_conf| sec_conf.to_section())
            .collect()
    }

    fn to_programs(&self, sections: &[SectionRef]) -> ConfigResult<Vec<ProgramRef>> {
        self.programs
            .iter()
            .map(|program_conf| program_conf.to_program(sections))
            .collect::<ConfigResult<_>>()
    }
}

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

    use std::fs::File;
    let file = File::open("config.json")
        .unwrap_or_else(|err| panic!("error opening config file: {}", err));
    let config: Config = serde_json::from_reader(file)
        .unwrap_or_else(|err| panic!("error parsing config file: {}", err));
    debug!("config: {:?}", config);

    let sections: Vec<SectionRef>;
    let programs: Vec<ProgramRef>;
    let section_runner = SectionRunner::start_new();
    let program_runner = ProgramRunner::start_new(section_runner.clone());

    info!("initializing sections");
    sections = config.to_sections();

    for section in &sections {
        section.set_state(false);
    }

    info!("initializing programs");

    programs = config.to_programs(&sections)
        .unwrap_or_else(|err| panic!("error deserializing programs: {}", err));

    for program in &programs {
        program_runner.schedule_program(program.clone());
    }

    Trap::trap(&[2, 15]).next(); // SIGINT, SIGKILL

    info!("received interrupt. stopping...");
    program_runner.stop();
    section_runner.stop();
}
