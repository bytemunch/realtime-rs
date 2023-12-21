#![allow(dead_code)]

use std::{
    collections::HashMap,
    env,
    net::TcpStream,
    sync::{
        mpsc::{self, Receiver, Sender},
        Arc, Mutex,
    },
    thread,
};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tungstenite::{
    client::IntoClientRequest, connect, http::HeaderValue, stream::MaybeTlsStream, Message,
    WebSocket,
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
pub(crate) enum Payload {
    Join(JoinPayload),
    Response(JoinResponsePayload),
    System(SystemPayload),
    AccessToken(AccessTokenPayload),
    PostgresChange(PostgresChangePayload),
    Empty {}, // TODO perf: implement custom deser cos this bad. typechecking: this matches
              // everything that can't deser elsewhere. not good.
}

#[derive(Serialize, Deserialize, Debug)]
pub(crate) struct PostgresChangePayload {
    data: PostgresChangeData,
    ids: Vec<usize>,
}

#[derive(Serialize, Deserialize, Debug)]
struct PostgresChangeData {
    columns: Vec<PostgresColumn>,
    commit_timestamp: String,
    errors: Option<String>,
    old_record: PostgresOldDataRef,
    record: HashMap<String, Value>,
    #[serde(rename = "type")]
    change_type: PostgresEvent,
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
pub(crate) struct AccessTokenPayload {
    access_token: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub(crate) struct SystemPayload {
    channel: String,
    extension: String,
    message: String,
    status: PayloadStatus,
}

#[derive(Serialize, Deserialize, Debug)]
pub(crate) struct JoinPayload {
    config: JoinConfig,
}

#[derive(Serialize, Deserialize, Debug)]
struct JoinConfig {
    broadcast: JoinConfigBroadcast,
    presence: JoinConfigPresence,
    postgres_changes: Vec<PostgresChange>,
}

#[derive(Serialize, Deserialize, Debug)]
struct JoinConfigBroadcast {
    #[serde(rename = "self")]
    broadcast_self: bool,
    ack: bool,
}

#[derive(Serialize, Deserialize, Debug)]
struct JoinConfigPresence {
    key: String,
}

#[derive(Serialize, Deserialize, Debug)]
enum PostgresEvent {
    #[serde(rename = "*")]
    All,
    #[serde(rename = "INSERT")]
    Insert,
    #[serde(rename = "UPDATE")]
    Update,
    #[serde(rename = "DELETE")]
    Delete,
}

#[derive(Serialize, Deserialize, Debug)]
struct PostgresChange {
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<isize>,
    event: PostgresEvent,
    schema: String,
    table: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    filter: Option<String>, // TODO structured filters
}

#[derive(Serialize, Deserialize, Debug)]
pub(crate) struct JoinResponsePayload {
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
pub(crate) struct RealtimeMessage {
    event: MessageEvent,
    topic: String,
    // TODO payload structure
    payload: Payload,
    #[serde(rename = "ref")]
    message_ref: Option<String>,
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
pub struct RealtimeClient {
    socket: Arc<Mutex<WebSocket<MaybeTlsStream<TcpStream>>>>,
    channels: Vec<RealtimeChannel>,
    message_queue: Vec<RealtimeMessage>,
    message_channel: (Sender<RealtimeMessage>, Receiver<RealtimeMessage>),
}

impl RealtimeClient {
    fn start() {}

    // alias to self::new(...);
    pub fn connect(local: bool, port: isize) -> RealtimeClient {
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
                port, local_anon_key
            )
        };

        let mut req = url.into_client_request().unwrap();
        let headers = req.headers_mut();

        let auth: HeaderValue = format!("Bearer {}", anon_key).parse().unwrap();
        headers.insert("Authorization", auth);

        let xci: HeaderValue = format!("realtime-rs/0.1.0").parse().unwrap();
        headers.insert("X-Client-Info", xci);

        println!("Connecting: {:?}", req);

        let conn = connect(req);

        let (socket, _res) = conn.expect("Uhoh connection broke");

        let socket = Arc::new(Mutex::new(socket));

        let message_channel = mpsc::channel::<RealtimeMessage>();

        // TODO check response.status is 101

        let init = RealtimeMessage {
            event: MessageEvent::Join,
            topic: format!("realtime:{}", "test"),
            payload: Payload::Join(JoinPayload {
                config: JoinConfig {
                    presence: JoinConfigPresence {
                        key: "test_key".to_owned(),
                    },
                    broadcast: JoinConfigBroadcast {
                        broadcast_self: true,
                        ack: false,
                    },
                    postgres_changes: vec![PostgresChange {
                        id: None,
                        event: PostgresEvent::All,
                        schema: "public".to_owned(),
                        table: "todos".to_owned(),
                        filter: None,
                    }],
                },
            }),
            message_ref: Some("init".to_owned()),
        };

        let join_message = serde_json::to_string(&init).expect("Json bad");
        println!("\nJoinMessage:\n{}\n", join_message);

        let _ = message_channel.0.send(init.into());

        // this needs to be in another thread.
        // loop {
        //     let msg = self.socket.read().expect("Error reading message");
        //
        //     let msg = msg.to_text().unwrap();
        //
        //     println!("Received: {}", msg);
        //     let v: RealtimeMessage = serde_json::from_str(msg).expect("Json like complex demon");
        //     println!("Value: {:?}", v);
        // }

        let loop_socket = Arc::clone(&socket);

        let tx = message_channel.0.clone();

        thread::spawn(move || loop {
            let msg = loop_socket
                .lock()
                .unwrap()
                .read()
                .expect("Error reading message");

            let msg = msg.to_text().unwrap();

            println!("Received: {}", msg);
            let v: RealtimeMessage = serde_json::from_str(msg).expect("Json like complex demon");
            println!("Value: {:?}", v);

            let _ = tx.send(v);
        });

        // TODO recieve loop for message_channel
        //
        for recieved in &message_channel.1 {
            println!("Got message in connect()! {:?}", recieved);
        }

        RealtimeClient {
            socket,
            channels: Vec::new(),
            message_queue: Vec::new(),
            message_channel,
        }

        // socket.close(None);
    }

    fn disconnect() {}

    fn heartbeat(&mut self) {
        match self.socket.lock() {
            Ok(mut socket) => {
                let _ = socket.send(RealtimeMessage::heartbeat().into());
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
            }
            Err(err) => {
                println!("mutex hard. {:?}", err);
            }
        };
    }

    fn on_message() {}
}
