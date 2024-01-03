#![allow(dead_code)]

use std::{
    collections::HashMap,
    env,
    net::TcpStream,
    sync::{
        mpsc::{self, Receiver, Sender},
        Arc, Mutex,
    },
    thread::{self, sleep},
    time::Duration,
};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tungstenite::{
    client::IntoClientRequest,
    connect,
    http::{HeaderMap, HeaderValue, StatusCode},
    stream::MaybeTlsStream,
    Message, WebSocket,
};

use crate::{constants::MessageEvent, realtime_channel::RealtimeChannel};

// this is where all the client structs and methods go :)
// pretty sure writing client first is the way to go
// but client needs channel
// and channel needs client
//
// client holds multiple channels
// client holds socket connection
// client forwards messages to correct channel(s)
// client manages heartbeat
//
// channels hold ref to their topic
// channels hold ref to presence
// channels forward events to callbacks
// channels register callbacks e.g.:
//
// &mut mychannel;
// fn filter_fn() {};
// fn do_something_rad(msg: RealtimePayload) {}
//  mychannel.on(EventType::Any, filter_fn, do_something_rad);
//
// presence is a black box to me right now i'll deal with that laterr

#[derive(Serialize, Deserialize, Debug)]
#[serde(untagged)]
pub enum Payload {
    Join(JoinPayload),
    Response(JoinResponsePayload),
    System(SystemPayload),
    AccessToken(AccessTokenPayload),
    PostgresChange(PostgresChangePayload), // TODO rename because clashes
    Empty {}, // TODO perf: implement custom deser cos this bad. typechecking: this matches
              // everything that can't deser elsewhere. not good.
}

#[derive(Serialize, Deserialize, Debug)]
pub struct PostgresChangePayload {
    pub data: PostgresChangeData,
    ids: Vec<usize>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct PostgresChangeData {
    columns: Vec<PostgresColumn>,
    commit_timestamp: String,
    errors: Option<String>,
    old_record: Option<PostgresOldDataRef>,
    record: Option<HashMap<String, Value>>,
    #[serde(rename = "type")]
    pub change_type: PostgresEvent,
    schema: String,
    table: String,
}

#[derive(Serialize, Deserialize, Debug)]
enum RecordValue {
    Bool(bool),
    Number(isize),
    String(String),
}

#[derive(Serialize, Deserialize, Debug)]
struct PostgresColumn {
    name: String,
    #[serde(rename = "type")]
    column_type: String,
}

#[derive(Serialize, Deserialize, Debug)]
struct PostgresOldDataRef {
    id: isize,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct AccessTokenPayload {
    access_token: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct SystemPayload {
    channel: String,
    extension: String,
    message: String,
    status: PayloadStatus,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct JoinPayload {
    pub config: JoinConfig,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct JoinConfig {
    pub broadcast: JoinConfigBroadcast,
    pub presence: JoinConfigPresence,
    pub postgres_changes: Vec<PostgresChange>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct JoinConfigBroadcast {
    #[serde(rename = "self")]
    pub(crate) broadcast_self: bool,
    pub(crate) ack: bool,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct JoinConfigPresence {
    pub key: String,
}

#[derive(Serialize, Deserialize, Debug, Eq, PartialEq, Default)]
pub enum PostgresEvent {
    #[serde(rename = "*")]
    #[default]
    All,
    #[serde(rename = "INSERT")]
    Insert,
    #[serde(rename = "UPDATE")]
    Update,
    #[serde(rename = "DELETE")]
    Delete,
}

#[derive(Serialize, Deserialize, Debug, Default)]
pub struct PostgresChange {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<isize>,
    pub event: PostgresEvent,
    pub schema: String,
    pub table: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filter: Option<String>, // TODO structured filters
}

#[derive(Serialize, Deserialize, Debug)]
pub struct JoinResponsePayload {
    response: JoinResponse,
    status: PayloadStatus,
}

#[derive(Serialize, Deserialize, Debug)]
struct JoinResponse {
    postgres_changes: Vec<PostgresChange>,
}

#[derive(Serialize, Deserialize, Debug)]
pub enum PayloadStatus {
    #[serde(rename = "ok")]
    Ok,
    #[serde(rename = "error")]
    Error,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct RealtimeMessage {
    pub event: MessageEvent,
    pub topic: String,
    // TODO payload structure
    pub payload: Payload,
    #[serde(rename = "ref")]
    pub message_ref: Option<String>,
}

impl RealtimeMessage {
    fn heartbeat() -> RealtimeMessage {
        RealtimeMessage {
            event: MessageEvent::Heartbeat,
            topic: "phoenix".to_owned(),
            payload: Payload::Empty {},
            message_ref: Some("".to_owned()),
        }
    }
}

impl Into<Message> for RealtimeMessage {
    fn into(self) -> Message {
        let data = serde_json::to_string(&self).expect("Uhoh cannot into message!");
        Message::Text(data)
    }
}

#[derive(Debug)]
pub struct RealtimeClientOptions {
    access_token: Option<String>,
    endpoint: String,
    headers: Option<HeaderMap>,
    params: Option<HashMap<String, String>>,
    timeout: Duration,
    heartbeat_interval: Duration,
    client_ref: isize,
    encode: Option<String>,               // placeholder
    decode: Option<String>,               // placeholder
    reconnect_interval: Option<Duration>, // placeholder TODO backoff function
}

impl Default for RealtimeClientOptions {
    fn default() -> Self {
        let mut headers = HeaderMap::new();
        headers.insert("X-Client-Info", "realtime-rs/0.1.0".parse().unwrap());

        Self {
            access_token: Some(env::var("ANON_KEY").unwrap()),
            endpoint: "ws://127.0.0.1:3012".to_string(),
            headers: Some(headers),
            params: None,
            timeout: Duration::from_millis(10000),
            heartbeat_interval: Duration::from_millis(30000),
            client_ref: 0,
            encode: None,
            decode: None,
            reconnect_interval: None,
        }
    }
}

#[derive(Debug)]
pub struct RealtimeClient {
    pub socket: Arc<Mutex<WebSocket<MaybeTlsStream<TcpStream>>>>,
    channels: HashMap<String, RealtimeChannel>,
    inbound_channel: (Sender<RealtimeMessage>, Receiver<RealtimeMessage>),
    /// Options to be used in internal fns
    options: RealtimeClientOptions,
}

impl RealtimeClient {
    fn start() {}

    // Test fn
    pub fn connect(local: bool, port: Option<isize>) -> RealtimeClient {
        let supabase_id = env::var("SUPABASE_ID").unwrap();
        let anon_key = env::var("ANON_KEY").unwrap();
        let _service_key = env::var("SERVICE_KEY").unwrap();

        let local_anon_key = env::var("LOCAL_ANON_KEY").unwrap();
        let _local_service_key = env::var("LOCAL_SERVICE_KEY").unwrap();

        let url = if !local {
            format!(
                "wss://{}.supabase.co/realtime/v1/websocket?apikey={}&vsn=1.0.0",
                supabase_id, anon_key
            )
        } else {
            format!(
                "ws://127.0.0.1:{}/realtime/v1/websocket?apikey={}&vsn=1.0.0",
                port.unwrap(),
                local_anon_key
            )
        };

        let mut req = url.into_client_request().unwrap();
        let headers = req.headers_mut();

        let auth: HeaderValue = format!("Bearer {}", anon_key).parse().unwrap();
        headers.insert("Authorization", auth);

        let xci: HeaderValue = format!("realtime-rs/0.1.0").parse().unwrap();
        headers.insert("X-Client-Info", xci);

        println!("Connecting... Req: {:?}\n", req);

        let conn = connect(req);

        let (socket, res) = conn.expect("Uhoh connection broke");

        if res.status() != StatusCode::SWITCHING_PROTOCOLS {
            panic!("101 code not recieved.");
        }

        let socket = Arc::new(Mutex::new(socket));

        let inbound_channel = mpsc::channel::<RealtimeMessage>();

        let loop_socket = Arc::clone(&socket);

        let inbound_tx = inbound_channel.0.clone();

        let _socket_thread = thread::spawn(move || loop {
            // Recieve from server
            let mut socket = loop_socket.lock().unwrap();
            let raw_message = socket.read().expect("Error reading message");
            // TODO lock held here if nothing to read.
            drop(socket);

            let msg = raw_message.to_text().unwrap();

            let msg: RealtimeMessage = serde_json::from_str(msg).expect("Json like complex demon");

            // TODO error handling; match on msg values
            match msg.payload {
                Payload::Empty {} => {
                    println!(
                        "Possibly malformed payload: {:?}",
                        raw_message.to_text().unwrap()
                    )
                }
                _ => {}
            }
            let _ = inbound_tx.send(msg);
        });

        let heartbeat_socket = Arc::clone(&socket);

        let _heartbeat_thread = thread::spawn(move || loop {
            // sleep thread
            sleep(Duration::from_secs(1));

            println!("Heartbeat.");

            match heartbeat_socket.lock() {
                Ok(mut socket) => {
                    let _ = socket.send(RealtimeMessage::heartbeat().into());
                    drop(socket);
                }
                Err(err) => {
                    println!("mutex hard. {:?}", err);
                }
            };
        });

        println!("Socket ready!");

        RealtimeClient {
            socket,
            channels: HashMap::new(),
            inbound_channel,
            options: RealtimeClientOptions::default(),
        }

        // socket.close(None);
    }

    fn disconnect() {}

    fn heartbeat(&mut self) {
        match self.socket.lock() {
            Ok(mut socket) => {
                let _ = socket.send(RealtimeMessage::heartbeat().into());
                drop(socket);
            }
            Err(err) => {
                println!("mutex hard. {:?}", err);
            }
        };
    }

    fn send(&mut self, msg: RealtimeMessage) {
        match self.socket.lock() {
            Ok(mut socket) => {
                let _ = socket.send(msg.into());
                drop(socket);
            }
            Err(err) => {
                println!("mutex hard. {:?}", err);
            }
        };
    }

    pub fn channel(&mut self, topic: String, changes: Vec<PostgresChange>) -> &mut RealtimeChannel {
        let topic = format!("realtime:{}", topic);

        let new_channel = RealtimeChannel::new(self, topic.clone(), changes);

        self.channels.insert(topic.clone(), new_channel);

        self.channels.get_mut(&topic).unwrap()
    }

    pub fn listen(&mut self) {
        for message in &self.inbound_channel.1 {
            let Some(c) = self.channels.get_mut(&message.topic) else {
                println!("Dropping off-topic message: {:?}", message);
                continue;
            };

            c.recieve(message);
        }
    }
}
