//! Contains MQTT related functionality

use super::*;
use mqttc::{PubSub, QoS, PubOpt, SubscribeTopic, TopicPath, Topic};
use std::{thread, result, io, fmt};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Weak;
use std::os::unix::thread::JoinHandleExt;
use std::net::ToSocketAddrs;
use std::any::Any;
use std::error::Error;
use std::collections::BTreeMap;
use serde_json::Value as JsonValue;
use util::duration_string;

struct LocalStorage(BTreeMap<mqttc::PacketIdentifier, Box<mqttc::Message>>);

impl LocalStorage {
    pub fn new() -> Box<LocalStorage> {
        Box::new(LocalStorage(BTreeMap::new() as BTreeMap<mqttc::PacketIdentifier,
                                                          Box<mqttc::Message>>))
    }
}

impl mqttc::store::Store for LocalStorage {
    fn put(&mut self, message: Box<mqttc::Message>) -> mqttc::store::Result<()> {
        self.0.insert(message.pid.unwrap(), message);
        Ok(())
    }

    fn get(&mut self, pid: mqttc::PacketIdentifier) -> mqttc::store::Result<Box<mqttc::Message>> {
        match self.0.get(&pid) {
            Some(m) => Ok(m.clone()),
            None => Err(mqttc::store::Error::NotFound(pid)),
        }
    }

    fn delete(&mut self, pid: mqttc::PacketIdentifier) -> mqttc::store::Result<()> {
        self.0.remove(&pid);
        Ok(())
    }
}

#[derive(Serialize, Deserialize, Debug)]
struct ApiResponse {
    message: String,
    data: JsonValue,
}

impl ApiResponse {
    fn new<S: Into<String>>(message: S, data: JsonValue) -> Self {
        ApiResponse {
            message: message.into(),
            data: data,
        }
    }
}

quick_error! {
    #[derive(Debug)]
    enum ApiError {
        InvalidPath(path: String) {
            description("message received on invalid path")
            display("message received on invalid path: {}", path)
        }
        InvalidPayloadUtf8(err: std::str::Utf8Error) {
            from()
            description("invalid utf8 bytes in payload")
            cause(err)
        }
        InvalidPayloadJson(err: serde_json::Error, method: &'static str) {
            description("invalid json in payload")
            display("invalid json in payload for request type {}", method)
            cause(err)
        }
        SectionNotFound(section_idx: usize) {
            description("section not found")
            display(self_) -> ("{}: {}", self_.description(), section_idx)
        }
    }
}

impl ApiError {
    fn as_code(&self) -> ResponseCode {
        use self::ApiError::*;
        match *self {
            InvalidPath(_) => ResponseCode::InvalidPath,
            InvalidPayloadUtf8(_) => ResponseCode::InvalidPayloadUtf8,
            InvalidPayloadJson(_, _) => ResponseCode::InvalidPayloadJson,
            SectionNotFound(_) => ResponseCode::SectionNotFound,
        }
    }
}

/// A code that can be returned from an rinklers API request
pub enum ResponseCode {
    /// The request completed sucessfully
    Success = 1000,
    /// The request was made with on an invalid topic path
    InvalidPath = 2000,
    /// The request was made with an invalid utf8 string in the payload
    InvalidPayloadUtf8 = 2001,
    /// The request was made with invalid json data for the specified request in the payload
    InvalidPayloadJson = 2002,
    /// The request was made on a section that was not found
    SectionNotFound = 2003,
}

#[derive(Serialize, Deserialize, Debug)]
struct ApiResponseData {
    message: String,
    code: usize,
    data: JsonValue,
}

impl ApiResponseData {
    fn new<S: Into<String>>(message: S, code: usize, data: JsonValue) -> Self {
        ApiResponseData {
            message: message.into(),
            code: code,
            data: data,
        }
    }
}

impl From<ApiError> for ApiResponseData {
    fn from(error: ApiError) -> Self {
        Self::new(format!("{}", error),
                  error.as_code() as usize,
                  JsonValue::Null)
    }
}

impl From<ApiResponse> for ApiResponseData {
    fn from(response: ApiResponse) -> Self {
        Self::new(response.message, 1000, response.data)
    }
}

/// Initializes a new `mqttc::Client` on the specified prefix
fn init_client<A: ToSocketAddrs>(prefix: &str, address: A) -> mqttc::Result<mqttc::Client> {
    let mut opts = mqttc::ClientOptions::new();
    let connected_path = format!("{}connected", prefix);
    try!(opts.set_last_will(connected_path,
                            "false".to_string(),
                            PubOpt::at_least_once() | PubOpt::retain()));
    opts.set_clean_session(true);
    opts.set_outgoing_store(LocalStorage::new());

    let net_opts = mqttc::NetworkOptions::new();

    opts.connect(address, net_opts)
}

fn run_on_client(mut client: mqttc::Client,
                 prefix: &str,
                 state: &Arc<State>,
                 running: &Arc<AtomicBool>) {
    let prefix_path = TopicPath::from_str(&prefix).unwrap();

    initial_pubs(&mut client, &prefix, &state);

    client.subscribe(SubscribeTopic {
            topic_path: format!("{}sections/+/set_state", prefix),
            qos: QoS::ExactlyOnce,
        })
        .unwrap();

    let response_topic = format!("{}responses", prefix);
    while running.load(Ordering::SeqCst) {
        let message = match client.await() {
            Ok(message) => message,
            Err(mqttc::Error::Disconnected) => {
                warn!("mqtt client disconnected");
                break;
            }
            Err(err) => {
                error!("mqtt client error: {}", err);
                break;
            }
        };
        if let Some(msg) = message {
            let result = process_msg(&prefix_path, &state, msg);
            let response_data: ApiResponseData = match result {
                Ok(response) => {
                    trace!("successfully completed request. response: {:?}", response);
                    response.into()
                }
                Err(error) => {
                    trace!("error completing request: {:?}", error);
                    error.into()
                }
            };
            let data = serde_json::to_string(&response_data).unwrap();
            client.publish(response_topic.clone(), data, PubOpt::exactly_once())
                .unwrap()
        }
    }

    match client.disconnect() {
        Ok(()) => debug!("disconnected from mqtt broker"),
        Err(err) => error!("error disconnecting from mqtt broker: {}", err),
    }
}

fn initial_pubs(client: &mut mqttc::Client, prefix: &str, state: &Arc<State>) {
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
}

fn process_msg(prefix_path: &TopicPath,
               state: &Arc<State>,
               msg: Box<mqttc::Message>)
               -> Result<ApiResponse, ApiError> {
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
            return Err(ApiError::InvalidPath(msg.topic.path()));
        }
    };

    let payload_str = try!(str::from_utf8(&*msg.payload));

    trace!("received mqtt message: topic: {:?}, payload: {}",
           topics,
           payload_str);

    let collection_name: Option<&str> = topics.get(0).cloned();
    let index: Option<usize> = topics.get(1)
        .and_then(|idx_str| usize::from_str_radix(idx_str, 10).ok());
    let request_type = topics.get(2).cloned();

    macro_rules! unwrap_payload {
            ($payload_type:ty : $for_method:expr) => (
                match serde_json::from_str::<$payload_type>(payload_str) {
                    Ok(payload) => payload,
                    Err(err) => {
                        return Err(ApiError::InvalidPayloadJson(err, $for_method));
                    }
                }
            )
       }

    match (collection_name, index, request_type) {
        (Some("sections"), Some(idx), Some("set_state")) => {
            let section = match state.sections.get(idx) {
                Some(section) => section,
                None => {
                    return Err(ApiError::SectionNotFound(idx));
                }
            };
            let sec_state: bool = unwrap_payload!(bool : "set_state");
            section.set_state(sec_state);
            Ok(ApiResponse::new(format!("setting section {} state to {}", idx, sec_state),
                                JsonValue::Null))
        }
        _ => Err(ApiError::InvalidPath(msg.topic.path())),
    }
}

/// A struct that exposes an API for manipulating this rinklers instance through an MQTT broker
pub struct MqttApi {
    join_handle: thread::JoinHandle<()>, // state: Arc<State>,
    running: Weak<AtomicBool>,
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
pub type MqttApiResult<T> = result::Result<T, MqttApiError>;

impl MqttApi {
    /// Creates a new `MqttApi` instance.
    /// `prefix` is the string that is prefixed to all topics exposed on the broker
    /// `client` is the `mqttc::Client` to use to expose the topics
    /// `state` is an `Arc` to the application state that is used by the apis
    pub fn start_new<S, A>(prefix: S, address: A, state: Arc<State>) -> Self
        where S: Into<String>,
              A: ToSocketAddrs + fmt::Debug + Send + 'static
    {
        let running = Arc::new(AtomicBool::new(true));
        let running_handle = Arc::downgrade(&running);
        let join_handle = {
            let prefix = prefix.into();
            thread::spawn(move || {
                Self::run(prefix, address, state, running);
            })
        };
        MqttApi {
            join_handle: join_handle,
            running: running_handle,
        }
    }

    /// Stops the `MqttApi`, disconnecting the client and waiting for the processing thread to
    /// quit.
    pub fn stop(self) -> MqttApiResult<()> {
        let running = match self.running.upgrade() {
            Some(running) => running,
            None => {
                // thread has already exited and dropped the `Arc`
                return Ok(());
            }
        };
        running.store(false, Ordering::SeqCst);
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

    fn run<A>(prefix: String, address: A, state: Arc<State>, running: Arc<AtomicBool>)
        where A: ToSocketAddrs
    {
        let socket_addrs: Vec<_> = match address.to_socket_addrs() {
            Ok(socket_addrs) => socket_addrs.collect(),
            Err(err) => {
                error!("invalid broker address specified: {}", err);
                return;
            }
        };
        while running.load(Ordering::SeqCst) {
            debug!("connecting to mqtt broker at {:?}", &socket_addrs);
            let client = match init_client(prefix.as_ref(), socket_addrs.as_slice()) {
                Ok(client) => client,
                Err(err) => {
                    let retry_duration = Duration::new(1, 0);
                    warn!("error connecting to mqtt broker: {}. retrying after {}",
                          err, duration_string(&retry_duration));
                    thread::sleep(retry_duration);
                    continue;
                }
            };
            info!("connected to mqtt broker");

            run_on_client(client, &prefix, &state, &running);
        }
    }
}
