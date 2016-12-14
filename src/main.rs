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
extern crate mqttc;
extern crate libc;
extern crate nix;

pub mod section;
pub mod section_runner;
pub mod program;
pub mod program_runner;
pub mod schedule;
pub mod schedule_runner;
pub mod util;
pub mod config;
pub mod mqtt_api;

pub use section::{Section, SectionRef, LogSection};
pub use section_runner::{SectionRunner, RunNotifier, RunNotification};
pub use program::{ProgItem, Program, ProgramRef};
pub use program_runner::ProgramRunner;
pub use schedule::{DateTimeBound, Schedule, every_day};
pub use schedule_runner::{ScheduleRunner, ScheduleGuard};
pub use config::{ConfigResult, Config, SectionConfig};
pub use mqtt_api::MqttApi;

use std::time::Duration;
use std::{env, str};
use std::sync::{Arc, Mutex};
use signal::trap::Trap;

/// The global state of the rinklers application
pub struct State {
    /// All of the sections available to this rinklers instance
    pub sections: Vec<SectionRef>,
    /// All of the programs defined in this rinklers instance
    pub programs: Vec<ProgramRef>,
    /// The `SectionRunner` for running sections (locked by a mutex so State is Sync)
    pub section_runner: Mutex<SectionRunner>,
    /// The `ProgramRunner` for running programs
    pub program_runner: ProgramRunner,
}

#[allow(dead_code)]
fn assert_impls() {
    fn assert_send<T: Send>() {}
    fn assert_sync<T: Sync>() {}

    assert_send::<State>();
    assert_sync::<State>();
}

impl State {
    /// Creates a new `State` by reading from a `Config`
    pub fn from_config(config: Config) -> ConfigResult<Self> {
        let sections: Vec<SectionRef> = config.to_sections();
        let programs: Vec<ProgramRef> = try!(config.to_programs(&sections));
        let section_runner = SectionRunner::start_new();
        let program_runner = ProgramRunner::start_new(section_runner.clone());

        let state = State {
            sections: sections,
            programs: programs,
            section_runner: Mutex::new(section_runner),
            program_runner: program_runner,
        };
        Ok(state)
    }

    /// Stops the `SectionRunner` and `ProgramRunner` associated with this state, consuming the
    /// state. This is drops the state.
    pub fn stop_all(self) {
        self.section_runner
            .into_inner()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .stop();
        self.program_runner.stop();
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
    let config_path = "config.json";
    debug!("loading config from {}", config_path);
    let file = File::open(config_path)
        .unwrap_or_else(|err| panic!("error opening config file: {}", err));
    let config: Config = serde_json::from_reader(file)
        .unwrap_or_else(|err| panic!("error parsing config file: {}", err));
    trace!("config: {:?}", config);

    let state = State::from_config(config)
        .unwrap_or_else(|err| panic!("error creating state: {}", err));
    let state = Arc::new(state);

    info!("initializing sections");
    for section in &state.sections {
        section.set_state(false);
    }

    info!("initializing programs");
    for program in &state.programs {
        state.program_runner.schedule_program(program.clone());
    }

    let prefix = "rinklers/";
    let addr = ("localhost", 1883);
    let mqtt_api = MqttApi::start_new(prefix, addr, state.clone());

    Trap::trap(&[2, 15]).next(); // SIGINT, SIGKILL

    info!("received interrupt. stopping...");
    mqtt_api.stop()
        .unwrap_or_else(|err| panic!("error stopping mqtt api: {}", err));
    Arc::try_unwrap(state)
        .ok()
        .expect("something still has a reference to the global state. cannot stop")
        .stop_all();
}
