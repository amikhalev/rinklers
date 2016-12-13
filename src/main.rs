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

mod mqtt_api {
    use super::*;
    use mqttc::{PubSub, QoS, PubOpt, SubscribeTopic, TopicPath, Topic};
    use std::{thread, result, io};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::os::unix::thread::JoinHandleExt;
    use std::any::Any;

    pub fn init_client(prefix: &str) -> mqttc::Result<mqttc::Client> {
        let mut opts = mqttc::ClientOptions::new();
        let connected_path = format!("{}connected", prefix);
        try!(opts.set_last_will(connected_path,
                                "false".to_string(),
                                PubOpt::at_least_once() | PubOpt::retain()));

        let net_opts = mqttc::NetworkOptions::new();

        let addr = ("localhost", 1883);
        debug!("connecting to mqtt broker at {:?}", addr);
        opts.connect(addr, net_opts)
    }

    /// A struct that exposes an API for manipulating this rinklers instance through an MQTT broker
    pub struct MqttApi {
        join_handle: thread::JoinHandle<()>, // state: Arc<State>,
        running: Arc<AtomicBool>,
    }

    quick_error! {
        #[derive(Debug)]
        /// An error that can be returned from `MqttApi`
        pub enum MqttApiError {
            ThreadJoinError(err: Box<Any + Send + 'static>) {
                description("there was an error waiting for the mqtt thread to quit")
            }
        }
    }

    /// A result that can be returned from `MqttApi` methods
    pub type Result<T> = result::Result<T, MqttApiError>;

    impl MqttApi {
        /// Creates a new `MqttApi` instance.
        /// `prefix` is the string that is prefixed to all topics exposed on the broker
        /// `client` is the `mqttc::Client` to use to expose the topics
        /// `state` is an `Arc` to the application state that is used by the apis
        pub fn new<S: Into<String>>(prefix: S, client: mqttc::Client, state: Arc<State>) -> Self {
            let running = Arc::new(AtomicBool::new(true));
            let join_handle = {
                let prefix = prefix.into();
                let state = state.clone();
                let running = running.clone();
                thread::spawn(move || {
                    Self::run(prefix, client, state, running);
                })
            };
            MqttApi {
                join_handle: join_handle,
                running: running,
            }
        }

        /// Stops the `MqttApi`, disconnecting the client and waiting for the processing thread to
        /// quit.
        pub fn stop(self) -> Result<()> {
            self.running.store(false, Ordering::SeqCst);
            {
                let pthread = self.join_handle.as_pthread_t();
                unsafe {
                    // SIGINT, so it should interrupt the thread and any syscalls should result
                    // in EINT
                    let ret = libc::pthread_kill(pthread, 2);
                    if ret != 0 {
                        panic!("libc::pthread_kill() error: {}",
                               io::Error::from_raw_os_error(ret));
                    }
                }
            }
            self.join_handle
                .join()
                .map_err(|err| MqttApiError::ThreadJoinError(err))
        }

        fn run(prefix: String,
               mut client: mqttc::Client,
               state: Arc<State>,
               running: Arc<AtomicBool>) {
            let prefix_path = TopicPath::from_str(&prefix).expect("invalid prefix path");

            let pubopt = PubOpt::at_least_once() | PubOpt::retain();
            client.publish(format!("{}connected", prefix), "true", pubopt)
                .unwrap();

            let sections: Vec<usize> = (0..state.sections.len()).collect();
            let sections = serde_json::to_string(&sections).unwrap();
            client.publish(format!("{}sections", prefix), sections, pubopt).unwrap();

            for (i, section) in state.sections.iter().enumerate() {
                let topic = format!("{}sections/{}", prefix, i);
                let section_config = SectionConfig::from_section(&section);
                let data = serde_json::to_string(&section_config).unwrap();
                client.publish(topic, data, pubopt).unwrap();

                let topic = format!("{}sections/{}/state", prefix, i);
                let state = section.state();
                let data = serde_json::to_string(&state).unwrap();
                client.publish(topic, data, pubopt).unwrap();
            }

            client.subscribe(SubscribeTopic {
                    topic_path: format!("{}sections/+/set_state", prefix),
                    qos: QoS::ExactlyOnce,
                })
                .unwrap();

            while running.load(Ordering::SeqCst) {
                if let Some(msg) = client.await().unwrap() {
                    let topics: Option<Vec<&str>> = msg.topic
                        .topics()
                        .iter()
                        .skip(prefix_path.len() - 1)
                        .map(|topic| match *topic {
                            Topic::Normal(ref topic) => Some(topic.as_str()),
                            _ => None,
                        })
                        .collect();
                    let topics = match topics {
                        Some(topics) => topics,
                        _ => {
                            warn!("strange path received with non-Topic::Normal components: {}",
                                  msg.topic.path());
                            continue;
                        }
                    };

                    let payload_str = match str::from_utf8(&*msg.payload) {
                        Ok(str) => str,
                        Err(err) => {
                            warn!("received message with invalid utf8 payload: {:?}", err);
                            continue;
                        }
                    };

                    trace!("received mqtt message: topic: {:?}, payload: {}",
                           topics,
                           payload_str);

                    let collection_name: Option<&str> = topics.get(0).cloned();
                    let index: Option<usize> = topics.get(1)
                        .and_then(|idx_str| usize::from_str_radix(idx_str, 10).ok());
                    let postfix = topics.get(2).cloned();

                   macro_rules! unwrap_payload {
                        ($payload_type:ty : $for_method:expr) => (
                            match serde_json::from_str::<$payload_type>(payload_str) {
                                Ok(payload) => payload,
                                Err(_) => {
                                    warn!("invalid payload for {}: \"{}\"", $for_method, payload_str);
                                    continue;
                                }
                            }
                        )
                   }

                    match (collection_name, index, postfix) {
                        (Some("sections"), Some(idx), Some("set_state")) => {
                            let sec_state: bool = unwrap_payload!(bool : "set_state");
                            debug!("setting section {} state to {}", idx, sec_state);
                            match state.sections.get(idx) {
                                Some(ref section) => section.set_state(sec_state),
                                None => warn!("section index out of range: {}", idx),
                            };
                        }
                        _ => debug!("received message on invalid topic {}", msg.topic.path()),
                    }
                }
            }

            client.disconnect()
                .unwrap_or_else(|err| panic!("error disconnecting from mqtt broker: {}", err));
            debug!("disconnected from mqtt broker")
        }
    }
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
    debug!("config: {:?}", config);

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
    let client = mqtt_api::init_client(prefix)
        .unwrap_or_else(|err| panic!("error connecting to mqtt broker: {}", err));
    let mqtt_api = MqttApi::new(prefix, client, state.clone());
    info!("connected to mqtt broker");

    Trap::trap(&[2, 15]).next(); // SIGINT, SIGKILL

    info!("received interrupt. stopping...");
    mqtt_api.stop()
        .unwrap_or_else(|err| panic!("error stopping mqtt api: {}", err));
    Arc::try_unwrap(state)
        .ok()
        .expect("something still has a reference to the global state. cannot stop")
        .stop_all();
}
