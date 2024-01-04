#![allow(dead_code)]

use std::{
    collections::HashMap,
    env, io,
    net::{SocketAddr, TcpStream, ToSocketAddrs},
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
    client::{uri_mode, IntoClientRequest},
    client_tls,
    error::UrlError,
    handshake::MidHandshake,
    http::{HeaderMap, HeaderValue, Response, StatusCode, Uri},
    stream::{MaybeTlsStream, Mode, NoDelay},
    ClientHandshake, Error, HandshakeError, Message, WebSocket,
};

use crate::{constants::MessageEvent, realtime_channel::RealtimeChannel};

// TODO move structs to own file
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
        let data = serde_json::to_string(&self).expect("Uhoh cannot into message");
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

        let uri: Uri = if !local {
            format!(
                "wss://{}.supabase.co/realtime/v1/websocket?apikey={}&vsn=1.0.0",
                supabase_id, anon_key
            )
            .parse()
            .unwrap()
        } else {
            format!(
                "ws://127.0.0.1:{}/realtime/v1/websocket?apikey={}&vsn=1.0.0",
                port.unwrap(),
                local_anon_key
            )
            .parse()
            .unwrap()
        };

        let mut request = uri.clone().into_client_request().unwrap();
        let headers = request.headers_mut();

        let auth: HeaderValue = format!("Bearer {}", anon_key).parse().unwrap();
        headers.insert("Authorization", auth);

        let xci: HeaderValue = format!("realtime-rs/0.1.0").parse().unwrap();
        headers.insert("X-Client-Info", xci);

        println!("Connecting... Req: {:?}\n", request);

        let uri = request.uri();
        let mode = uri_mode(uri).expect("URI mode broke.");
        let host = request
            .uri()
            .host()
            .ok_or(Error::Url(UrlError::NoHostName))
            .expect("Malformed URI.");

        let host = if host.starts_with('[') {
            &host[1..host.len() - 1]
        } else {
            host
        };

        let port = uri.port_u16().unwrap_or(match mode {
            Mode::Plain => 80,
            Mode::Tls => 443,
        });

        let addrs = (host, port)
            .to_socket_addrs()
            .expect("Couldn't generate addrs");

        fn get_stream(addrs: &[SocketAddr], uri: &Uri) -> Result<TcpStream, ()> {
            for addr in addrs {
                println!("Trying to contact {} at {}...", uri, addr);
                if let Ok(stream) = TcpStream::connect(addr) {
                    return Ok(stream);
                }
            }

            Err(())
        }

        let mut stream =
            get_stream(addrs.as_slice(), uri).expect("Couldn't get stream from addrs.");

        NoDelay::set_nodelay(&mut stream, true).expect("Couldn't set NoDelay");

        stream
            .set_nonblocking(true)
            .expect("Couldn't set stream nonblocking");

        // TODO alias these types if they keep coming up
        fn retry_handshake(
            mid_hs: MidHandshake<ClientHandshake<MaybeTlsStream<TcpStream>>>,
        ) -> Result<
            (
                WebSocket<MaybeTlsStream<TcpStream>>,
                tungstenite::http::Response<Option<Vec<u8>>>,
            ),
            Error,
        > {
            match mid_hs.handshake() {
                Ok(stream) => {
                    return Ok(stream);
                }
                Err(e) => {
                    match e {
                        HandshakeError::Interrupted(mid_hs) => {
                            // recurse TODO is this a good idea? look into rust recursion
                            return retry_handshake(mid_hs);
                        }
                        HandshakeError::Failure(err) => {
                            return Err(err);
                        }
                    }
                }
            }
        }

        fn get_connection<R>(
            request: R,
            stream: TcpStream,
        ) -> Result<
            (
                WebSocket<MaybeTlsStream<TcpStream>>,
                Response<Option<Vec<u8>>>,
            ),
            Error,
        >
        where
            R: IntoClientRequest,
        {
            match client_tls(request, stream) {
                Ok(stream) => Ok(stream),
                Err(err) => match err {
                    HandshakeError::Failure(err) => Err(err),
                    HandshakeError::Interrupted(mid_hs) => return retry_handshake(mid_hs),
                },
            }
        }

        let conn = get_connection(request, stream).expect("Connection failed");

        let (socket, res) = conn;

        if res.status() != StatusCode::SWITCHING_PROTOCOLS {
            panic!("101 code not recieved.");
        }

        let socket = Arc::new(Mutex::new(socket));

        let inbound_channel = mpsc::channel::<RealtimeMessage>();

        let loop_socket = Arc::clone(&socket);

        let inbound_tx = inbound_channel.0.clone();

        let _socket_thread = thread::spawn(move || loop {
            // Recieve from server
            // TODO nested way too much for my liking
            match loop_socket.lock() {
                Ok(mut socket) => {
                    match socket.read() {
                        Ok(raw_message) => {
                            let msg = raw_message.to_text().unwrap();

                            let msg: RealtimeMessage =
                                serde_json::from_str(msg).expect("Deserialization error: ");

                            // TODO error handling; match on all msg values
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
                        }
                        Err(Error::Io(err)) if err.kind() == io::ErrorKind::WouldBlock => {
                            // do nothing here :)
                        }
                        Err(err) => {
                            println!("Socket read error: {:?}", err);
                        }
                    }

                    drop(socket);
                }
                Err(err) => {
                    println!("Socket thread: mutex hard. {:?}", err);
                }
            }
        });

        let heartbeat_socket = Arc::clone(&socket);

        let _heartbeat_thread = thread::spawn(move || loop {
            // sleep thread
            sleep(Duration::from_secs(29));

            match heartbeat_socket.lock() {
                Ok(mut socket) => {
                    let _ = socket.send(RealtimeMessage::heartbeat().into());
                    drop(socket);
                }
                Err(err) => {
                    println!("Heartbeat thread: mutex hard. {:?}", err);
                }
            };
        });

        println!("Socket ready");

        RealtimeClient {
            socket,
            channels: HashMap::new(),
            inbound_channel,
            options: RealtimeClientOptions::default(),
        }

        // socket.close(None);
    }

    fn disconnect() {
        // TODO
    }

    pub fn send(&mut self, msg: RealtimeMessage) {
        match self.socket.lock() {
            Ok(mut socket) => {
                println!("Sending: {:?}", msg);
                let _ = socket.send(msg.into());
                drop(socket);
            }
            Err(err) => {
                println!("Send: mutex hard. {:?}", err);
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
