//! Contains MQTT related functionality

use super::*;
use mqttc::{PubSub, QoS, PubOpt, SubscribeTopic, TopicPath, Topic};
use std::{thread, result, io};
use std::sync::atomic::{AtomicBool, Ordering};
use std::os::unix::thread::JoinHandleExt;
use std::net::ToSocketAddrs;
use std::any::Any;

/// Initializes a new `mqttc::Client` on the specified prefix
pub fn init_client<A: ToSocketAddrs>(prefix: &str, address: A) -> mqttc::Result<mqttc::Client> {
    let mut opts = mqttc::ClientOptions::new();
    let connected_path = format!("{}connected", prefix);
    try!(opts.set_last_will(connected_path,
                            "false".to_string(),
                            PubOpt::at_least_once() | PubOpt::retain()));

    let net_opts = mqttc::NetworkOptions::new();

    opts.connect(address, net_opts)
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
        /// An error that occurs when joining on the `MqttApi` thread
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
    pub fn new<S: AsRef<str>>(prefix: S, client: mqttc::Client, state: Arc<State>) -> Self {
        let running = Arc::new(AtomicBool::new(true));
        let join_handle = {
            let prefix = prefix.as_ref().to_string();
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

    /// Connects to the mqtt broker at `address` and creates a new `MqttApi` instance based on
    /// the connected client.
    /// # See [new](#fn.new)
    pub fn new_connect<S: AsRef<str>, A: ToSocketAddrs>(prefix: S,
                                                        address: A,
                                                        state: Arc<State>)
                                                        -> mqttc::Result<Self> {
        let client = try!(init_client(prefix.as_ref(), address));
        Ok(Self::new(prefix, client, state))
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
                Self::process_msg(&prefix_path, &state, msg);
            }
        }

        client.disconnect()
            .unwrap_or_else(|err| panic!("error disconnecting from mqtt broker: {}", err));
        debug!("disconnected from mqtt broker")
    }

    fn process_msg(prefix_path: &TopicPath, state: &Arc<State>, msg: Box<mqttc::Message>) {
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
                return;
            }
        };

        let payload_str = match str::from_utf8(&*msg.payload) {
            Ok(str) => str,
            Err(err) => {
                warn!("received message with invalid utf8 payload: {:?}", err);
                return;
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
                            return;
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
