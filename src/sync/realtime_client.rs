#![allow(dead_code)]

use std::fmt::{Debug, Display};
use std::thread::sleep;
use std::time::SystemTime;
use std::{
    collections::HashMap,
    io,
    net::{TcpStream, ToSocketAddrs},
    sync::mpsc::{self, Receiver, Sender, TryRecvError},
    time::Duration,
};

use native_tls::TlsConnector;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tungstenite::{client, Message};
use tungstenite::{
    client::{uri_mode, IntoClientRequest},
    handshake::MidHandshake,
    http::{HeaderMap, HeaderValue, Response as HttpResponse, StatusCode, Uri},
    stream::{MaybeTlsStream, Mode, NoDelay},
    ClientHandshake, Error, HandshakeError, WebSocket as WebSocketWrapper,
};
use uuid::Uuid;

use crate::message::payload::Payload;
use crate::message::realtime_message::RealtimeMessage;
use crate::sync::realtime_channel::{ChannelState, RealtimeChannel};

use super::realtime_channel::RealtimeChannelBuilder;

pub type Response = HttpResponse<Option<Vec<u8>>>;
pub type WebSocket = WebSocketWrapper<MaybeTlsStream<TcpStream>>;

#[derive(PartialEq, Debug, Default)]
pub enum ConnectionState {
    /// Client wants to reconnect
    Reconnect,
    /// Client is mid-reconnect
    Reconnecting,
    Connecting,
    Open,
    Closing,
    #[default]
    Closed,
}

#[derive(PartialEq, Debug)]
pub enum NextMessageError {
    WouldBlock,
    TryRecvError(TryRecvError),
    NoChannel,
    ChannelClosed,
    ClientClosed,
    SocketError(SocketError),
    MonitorError(MonitorError),
}

impl std::error::Error for NextMessageError {}
impl Display for NextMessageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&format!("{:?}", self))
    }
}

#[derive(PartialEq, Debug)]
pub enum MonitorError {
    ReconnectError,
    MaxReconnects,
    WouldBlock,
    Disconnected,
}

#[derive(Debug, PartialEq)]
pub enum SocketError {
    NoSocket,
    NoRead,
    NoWrite,
    Disconnected,
    WouldBlock,
    TooManyRetries,
    HandshakeError,
}

#[derive(Debug, PartialEq)]
pub enum ConnectError {
    BadUri,
    BadHost,
    BadAddrs,
    StreamError,
    NoDelayError,
    NonblockingError,
    HandshakeError,
    MaxRetries,
    WrongProtocol,
}

#[derive(Debug, PartialEq)]
pub enum MonitorSignal {
    Reconnect,
}

#[derive(Serialize, Deserialize)]
pub struct AuthResponse {
    access_token: String,
    token_type: String,
    expires_in: usize,
    expires_at: usize,
    refresh_token: String,
    user: HashMap<String, Value>,
}

pub struct MessageChannel((Sender<RealtimeMessage>, Receiver<RealtimeMessage>));

impl Default for MessageChannel {
    fn default() -> Self {
        Self(mpsc::channel())
    }
}

pub struct MonitorChannel((Sender<MonitorSignal>, Receiver<MonitorSignal>));

impl Default for MonitorChannel {
    fn default() -> Self {
        Self(mpsc::channel())
    }
}

#[derive(Default)]
pub struct RealtimeClient {
    socket: Option<WebSocket>,
    channels: HashMap<Uuid, RealtimeChannel>,
    pub status: ConnectionState,
    pub access_token: String,
    messages_this_second: Vec<SystemTime>,
    // mpsc
    inbound_channel: MessageChannel,
    pub(crate) outbound_channel: MessageChannel,
    monitor_channel: MonitorChannel,
    middleware: HashMap<Uuid, Box<dyn Fn(RealtimeMessage) -> RealtimeMessage>>,
    // timers
    reconnect_now: Option<SystemTime>,
    reconnect_delay: Duration,
    reconnect_attempts: usize,
    heartbeat_now: Option<SystemTime>,
    // builder options
    headers: HeaderMap,
    params: Option<HashMap<String, String>>,
    heartbeat_interval: Duration,
    client_ref: Option<String>,
    encode: Option<String>, // placeholder
    decode: Option<String>, // placeholder
    reconnect_interval: ReconnectFn,
    reconnect_max_attempts: usize,
    connection_timeout: Duration,
    auth_url: Option<String>,
    endpoint: String,
    max_events_per_second: usize,
}

impl Debug for RealtimeClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // TODO this is horrid
        f.write_fmt(format_args!(
            "{:?} {:?} {:?} {:?}  {}",
            self.socket,
            self.channels,
            self.inbound_channel.0,
            self.outbound_channel.0,
            "TODO middleware debug fmt"
        ))
    }
}

impl RealtimeClient {
    pub fn builder(endpoint: String, access_token: String) -> RealtimeClientBuilder {
        RealtimeClientBuilder::new(endpoint, access_token)
    }

    pub fn channel(&mut self, topic: String) -> RealtimeChannelBuilder {
        RealtimeChannelBuilder::new(self).topic(topic)
    }

    pub fn get_channel_tx(&self) -> Sender<RealtimeMessage> {
        self.outbound_channel.0 .0.clone()
    }

    pub fn connect(&mut self) -> Result<&mut RealtimeClient, ConnectError> {
        let uri: Uri = match format!(
            "{}/realtime/v1/websocket?apikey={}&vsn=1.0.0",
            self.endpoint, self.access_token
        )
        .parse()
        {
            Ok(uri) => uri,
            Err(_e) => return Err(ConnectError::BadUri),
        };

        // TODO REFAC tidy
        let ws_scheme = match uri.scheme_str() {
            Some(scheme) => {
                if scheme == "http" {
                    "ws"
                } else {
                    "wss"
                }
            }
            None => "ws",
        };

        let uri = Uri::builder()
            .scheme(ws_scheme)
            .authority(uri.authority().unwrap().clone())
            .path_and_query(uri.path_and_query().unwrap().clone())
            .build()
            .unwrap();

        let Ok(mut request) = uri.clone().into_client_request() else {
            return Err(ConnectError::BadUri);
        };

        let headers = request.headers_mut();

        let auth: HeaderValue = format!("Bearer {}", self.access_token)
            .parse()
            .expect("malformed access token?");
        headers.insert("Authorization", auth);

        // unwrap: shouldn't fail
        let xci: HeaderValue = "realtime-rs/0.1.0".to_string().parse().unwrap();
        headers.insert("X-Client-Info", xci);

        println!("Connecting... Req: {:?}\n", request);

        let uri = request.uri();

        let Ok(mode) = uri_mode(uri) else {
            return Err(ConnectError::BadUri);
        };

        let Some(host) = uri.host() else {
            return Err(ConnectError::BadHost);
        };

        let port = uri.port_u16().unwrap_or(match mode {
            Mode::Plain => 80,
            Mode::Tls => 443,
        });

        let Ok(mut addrs) = (host, port).to_socket_addrs() else {
            return Err(ConnectError::BadAddrs);
        };

        let mut stream = match TcpStream::connect_timeout(
            &addrs.next().expect("uhoh no addr"),
            self.connection_timeout,
        ) {
            Ok(stream) => {
                self.reconnect_attempts = 0;
                stream
            }
            Err(_e) => {
                if self.reconnect_attempts < self.reconnect_max_attempts {
                    self.reconnect_attempts += 1;
                    let backoff = self.reconnect_interval.0.as_ref();
                    sleep(backoff(self.reconnect_attempts));
                    return self.connect();
                }
                // TODO get reason from stream error
                return Err(ConnectError::StreamError);
            }
        };

        let Ok(()) = NoDelay::set_nodelay(&mut stream, true) else {
            return Err(ConnectError::NoDelayError);
        };

        let maybe_tls = match mode {
            Mode::Tls => {
                let connector = TlsConnector::new().expect("No TLS tings");

                let connected_stream = connector
                    .connect(host, stream.try_clone().expect("noclone"))
                    .unwrap();

                stream
                    .set_nonblocking(true)
                    .expect("blocking mode oh nooooo");

                MaybeTlsStream::NativeTls(connected_stream)
            }
            Mode::Plain => {
                stream
                    .set_nonblocking(true)
                    .expect("blocking mode oh nooooo");

                MaybeTlsStream::Plain(stream)
            }
        };

        let conn: Result<(WebSocket, Response), Error> = match client(request, maybe_tls) {
            Ok(stream) => {
                self.reconnect_attempts = 0;
                Ok(stream)
            }
            Err(err) => match err {
                HandshakeError::Failure(_err) => {
                    // TODO DRY break reconnect attempts code into own fn
                    if self.reconnect_attempts < self.reconnect_max_attempts {
                        self.reconnect_attempts += 1;
                        let backoff = &self.reconnect_interval.0;
                        sleep(backoff(self.reconnect_attempts));
                        return self.connect();
                    }

                    return Err(ConnectError::HandshakeError);
                }
                HandshakeError::Interrupted(mid_hs) => match self.retry_handshake(mid_hs) {
                    Ok(stream) => Ok(stream),
                    Err(_err) => {
                        if self.reconnect_attempts < self.reconnect_max_attempts {
                            self.reconnect_attempts += 1;
                            let backoff = &self.reconnect_interval.0;
                            sleep(backoff(self.reconnect_attempts));
                            return self.connect();
                        }

                        // TODO err data
                        return Err(ConnectError::MaxRetries);
                    }
                },
            },
        };

        let (socket, res) = conn.expect("Handshake fail");

        if res.status() != StatusCode::SWITCHING_PROTOCOLS {
            return Err(ConnectError::WrongProtocol);
        }

        self.socket = Some(socket);

        self.status = ConnectionState::Open;

        Ok(self)
    }

    fn retry_handshake(
        &mut self,
        mid_hs: MidHandshake<ClientHandshake<MaybeTlsStream<TcpStream>>>,
    ) -> Result<(WebSocket, Response), SocketError> {
        match mid_hs.handshake() {
            Ok(stream) => Ok(stream),
            Err(e) => match e {
                HandshakeError::Interrupted(mid_hs) => {
                    // TODO sleeping main thread bad
                    if self.reconnect_attempts < self.reconnect_max_attempts {
                        self.reconnect_attempts += 1;
                        let backoff = &self.reconnect_interval.0;
                        sleep(backoff(self.reconnect_attempts));
                        return self.retry_handshake(mid_hs);
                    }

                    Err(SocketError::TooManyRetries)
                }
                HandshakeError::Failure(_err) => {
                    // TODO pass error data
                    Err(SocketError::HandshakeError)
                }
            },
        }
    }

    fn run_monitor(&mut self) -> Result<(), MonitorError> {
        if self.reconnect_now.is_none() {
            self.reconnect_now = Some(SystemTime::now());

            self.reconnect_delay = self.reconnect_interval.0(self.reconnect_attempts);
        }

        match self.monitor_channel.0 .1.try_recv() {
            Ok(signal) => match signal {
                MonitorSignal::Reconnect => {
                    if self.status == ConnectionState::Open
                        || self.status == ConnectionState::Reconnecting
                        || SystemTime::now() < self.reconnect_now.unwrap() + self.reconnect_delay
                    {
                        return Err(MonitorError::WouldBlock);
                    }

                    if self.reconnect_attempts >= self.reconnect_max_attempts {
                        return Err(MonitorError::MaxReconnects);
                    }

                    self.status = ConnectionState::Reconnecting;
                    self.reconnect_attempts += 1;
                    self.reconnect_now.take();

                    match self.connect() {
                        Ok(_) => {
                            for channel in self.channels.values_mut() {
                                channel.subscribe();
                            }

                            Ok(())
                        }
                        Err(e) => {
                            println!("reconnect error: {:?}", e);
                            self.status = ConnectionState::Reconnect;
                            Err(MonitorError::ReconnectError)
                        }
                    }
                }
            },
            Err(TryRecvError::Empty) => Err(MonitorError::WouldBlock),
            Err(TryRecvError::Disconnected) => Err(MonitorError::Disconnected),
        }
    }

    fn run_heartbeat(&mut self) {
        if self.heartbeat_now.is_none() {
            self.heartbeat_now = Some(SystemTime::now());
        }

        if self.heartbeat_now.unwrap() + self.heartbeat_interval > SystemTime::now() {
            return;
        }

        self.heartbeat_now.take();

        let _ = self.send(RealtimeMessage::heartbeat());
    }

    fn read_socket(&mut self) -> Result<(), SocketError> {
        let Some(ref mut socket) = self.socket else {
            return Err(SocketError::NoSocket);
        };

        if !socket.can_read() {
            return Err(SocketError::NoRead);
        }

        match socket.read() {
            Ok(raw_message) => match raw_message {
                Message::Text(string_message) => {
                    // TODO recoverable error
                    let message: RealtimeMessage =
                        serde_json::from_str(&string_message).expect("Deserialization error: ");

                    println!("[RECV] {:?}", message);

                    if let Payload::Empty {} = message.payload {
                        println!("Possibly malformed payload: {:?}", string_message)
                    }

                    let _ = self.inbound_channel.0 .0.send(message);
                    Ok(())
                }
                Message::Close(_close_message) => {
                    self.disconnect();
                    Err(SocketError::Disconnected)
                }
                _ => {
                    // do nothing on ping, pong, binary messages
                    Err(SocketError::WouldBlock)
                }
            },
            Err(Error::Io(err)) if err.kind() == io::ErrorKind::WouldBlock => {
                // do nothing here :)
                Ok(())
            }
            Err(err) => {
                println!("Socket read error: {:?}", err);
                self.status = ConnectionState::Reconnect;
                let _ = self.monitor_channel.0 .0.send(MonitorSignal::Reconnect);
                Err(SocketError::WouldBlock)
            }
        }
    }

    fn write_socket(&mut self) -> Result<(), SocketError> {
        let Some(ref mut socket) = self.socket else {
            return Err(SocketError::NoSocket);
        };

        if !socket.can_write() {
            return Err(SocketError::NoWrite);
        }

        // Throttling
        let now = SystemTime::now();

        self.messages_this_second = self
            .messages_this_second
            .clone() // TODO do i need this clone? can i mutate in-place?
            .into_iter()
            .filter(|st| now.duration_since(*st).unwrap_or_default() < Duration::from_secs(1))
            .collect();

        if self.messages_this_second.len() >= self.max_events_per_second {
            return Err(SocketError::WouldBlock);
        }

        // Send to server
        // TODO should drain outbound_channel
        // TODO drain should respect throttling
        let message = self.outbound_channel.0 .1.try_recv();

        match message {
            Ok(message) => {
                let raw_message = serde_json::to_string(&message);
                println!("[SEND] {:?}", raw_message);
                let _ = socket.send(message.into());
                self.messages_this_second.push(now);
                Ok(())
            }
            Err(TryRecvError::Empty) => {
                // do nothing
                Ok(())
            }
            Err(e) => {
                println!("outbound error: {:?}", e);
                self.status = ConnectionState::Reconnect;
                let _ = self.monitor_channel.0 .0.send(MonitorSignal::Reconnect);
                Err(SocketError::WouldBlock)
            }
        }
    }

    fn reconnect(&mut self) {
        self.status = ConnectionState::Reconnect;
        let _ = self.monitor_channel.0 .0.send(MonitorSignal::Reconnect);
    }

    pub fn disconnect(&mut self) {
        if self.status == ConnectionState::Closed {
            return;
        }

        self.remove_all_channels();

        self.status = ConnectionState::Closed;

        let Some(ref mut socket) = self.socket else {
            println!("Already disconnected. {:?}", self.status);
            return;
        };

        let _ = socket.close(None);
        println!("Client disconnected. {:?}", self.status);
    }

    pub fn send(&mut self, msg: RealtimeMessage) -> Result<(), mpsc::SendError<RealtimeMessage>> {
        self.outbound_channel.0 .0.send(msg)
    }

    pub(crate) fn add_channel(&mut self, channel: RealtimeChannel) -> Uuid {
        let id = channel.id;
        self.channels.insert(channel.id, channel);
        id
    }

    pub fn get_channel_mut(&mut self, channel_id: Uuid) -> &mut RealtimeChannel {
        self.channels.get_mut(&channel_id).unwrap()
    }

    pub fn get_channel(&self, channel_id: Uuid) -> &RealtimeChannel {
        self.channels.get(&channel_id).unwrap()
    }

    pub fn get_channels(&self) -> &HashMap<Uuid, RealtimeChannel> {
        &self.channels
    }

    pub fn remove_channel(&mut self, channel_id: Uuid) -> Option<RealtimeChannel> {
        if let Some(mut channel) = self.channels.remove(&channel_id) {
            let _ = channel.unsubscribe();

            if self.channels.is_empty() {
                self.disconnect();
            }

            return Some(channel);
        }

        None
    }

    pub fn remove_all_channels(&mut self) {
        if self.status == ConnectionState::Closing || self.status == ConnectionState::Closed {
            return;
        }

        self.status = ConnectionState::Closing;

        // wait until inbound_rx is drained
        loop {
            let recv = self.next_message();

            if Err(NextMessageError::WouldBlock) == recv {
                break;
            }
        }

        loop {
            let _ = self.next_message();

            let mut all_channels_closed = true;

            for channel in self.channels.values_mut() {
                let channel_state = channel.unsubscribe();

                match channel_state {
                    Ok(state) => {
                        if state != ChannelState::Closed {
                            all_channels_closed = false;
                        }
                    }
                    Err(e) => {
                        // TODO error handling
                        println!("Unsubscribe error: {:?}", e);
                    }
                }
            }

            if all_channels_closed {
                println!("All channels closed!");
                break;
            }
        }

        self.channels.clear();
    }

    pub fn block_until_subscribed(&mut self, channel_id: Uuid) -> Result<Uuid, ChannelState> {
        // TODO ergonomically this would fit better as a function on RealtimeChannel but borrow
        // checker

        let channel = self.channels.get_mut(&channel_id);

        let channel = channel.unwrap();

        if channel.status == ChannelState::Joined {
            return Ok(channel.id);
        }

        if channel.status != ChannelState::Joining {
            self.channels.get_mut(&channel_id).unwrap().subscribe();
        }

        loop {
            match self.next_message() {
                Ok(topic) => {
                    println!("[Blocking Subscribe] Message forwarded to {:?}", topic)
                }
                Err(NextMessageError::WouldBlock) => {}
                Err(_e) => {
                    //println!("NextMessageError: {:?}", e)
                }
            }

            let channel = self.channels.get_mut(&channel_id).unwrap();

            match channel.status {
                ChannelState::Joined => {
                    break;
                }
                ChannelState::Closed => return Err(ChannelState::Closed),
                _ => {}
            }
        }

        Ok(channel_id)
    }

    pub fn sign_in_with_email_password(&mut self, email: String, password: String) {
        let client = reqwest::blocking::Client::new(); //TODO one reqwest client per realtime client. or just like write gotrue-rs already
        let url = self.auth_url.clone().unwrap_or(self.endpoint.clone());

        let url = format!("{}/auth/v1", url);

        let res = client
            .post(format!("{}/token?grant_type=password", url))
            .header(CONTENT_TYPE, "application/json")
            .header(
                AUTHORIZATION,
                format!("Bearer {}", self.access_token.clone()),
            )
            .header("apikey", self.access_token.clone())
            .body(json!({"email": email, "password": password}).to_string())
            .send();

        if let Ok(res) = res {
            match res.json::<AuthResponse>() {
                Ok(res) => {
                    self.set_auth(res.access_token);
                    println!("Login success");
                    return;
                }
                Err(e) => {
                    println!("Login failed! Bad login? {:?}", e);
                    return;
                }
            }

            // TODO error, bad login
        }
        println!("Login failed! Malformed data?");
        return;
        // TODO error, malformed auth packet / url
    }

    pub fn set_auth(&mut self, access_token: String) {
        self.access_token = access_token.clone();

        for channel in self.channels.values_mut() {
            // TODO single source of data for access token
            let _ = channel.set_auth(access_token.clone()); // TODO error handling
        }
    }

    // TODO look into if middleware is needed?
    pub fn add_middleware(
        &mut self,
        middleware: Box<dyn Fn(RealtimeMessage) -> RealtimeMessage>,
    ) -> &mut RealtimeClient {
        // TODO user defined middleware ordering
        let uuid = Uuid::new_v4();
        self.middleware.insert(uuid, middleware);
        self
    }

    pub fn run_middleware(&self, mut message: RealtimeMessage) -> RealtimeMessage {
        for middleware in self.middleware.values() {
            message = middleware(message)
        }
        message
    }

    pub fn remove_middleware(&mut self, uuid: Uuid) -> &mut RealtimeClient {
        self.middleware.remove(&uuid);
        self
    }

    pub fn next_message(&mut self) -> Result<Vec<Uuid>, NextMessageError> {
        if self.status == ConnectionState::Closed {
            return Err(NextMessageError::ClientClosed);
        }

        if self.status == ConnectionState::Reconnect {
            self.status = ConnectionState::Reconnect;
            let _ = self.monitor_channel.0 .0.send(MonitorSignal::Reconnect);
            return Err(NextMessageError::SocketError(SocketError::Disconnected));
        }

        if self.status == ConnectionState::Reconnecting {
            return Err(NextMessageError::WouldBlock);
        }

        match self.run_monitor() {
            Ok(_) => {}
            Err(MonitorError::WouldBlock) => {}
            Err(MonitorError::MaxReconnects) => {
                self.disconnect();
                return Err(NextMessageError::MonitorError(MonitorError::MaxReconnects));
            }
            Err(e) => {
                return Err(NextMessageError::MonitorError(e));
            }
        }

        self.run_heartbeat();

        for channel in self.channels.values_mut() {
            let _ = channel.drain_queue();
            // all errors here are wouldblock, i think
        }

        match self.write_socket() {
            Ok(()) => {}
            Err(SocketError::WouldBlock) => {}
            Err(e) => {
                self.reconnect();
                return Err(NextMessageError::SocketError(e));
            }
        }

        match self.read_socket() {
            Ok(()) => {}
            Err(e) => {
                self.reconnect();
                return Err(NextMessageError::SocketError(e));
            }
        }

        let message = self.inbound_channel.0 .1.try_recv();

        match message {
            Ok(mut message) => {
                let mut ids = vec![];
                // TODO filter & route system messages and the like

                // Run middleware
                message = self.run_middleware(message);

                // Send message to channel
                for (id, channel) in &mut self.channels {
                    if channel.topic == message.topic {
                        channel.recieve(message.clone());
                        ids.push(*id);
                    }
                }

                Ok(ids)
            }
            Err(TryRecvError::Empty) => Err(NextMessageError::WouldBlock),
            Err(e) => Err(NextMessageError::TryRecvError(e)),
        }
    }
}

pub struct ReconnectFn(Box<dyn Fn(usize) -> Duration>);

impl Default for ReconnectFn {
    fn default() -> Self {
        Self(Box::new(backoff))
    }
}

impl Debug for ReconnectFn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TODO reconnect fn debug")
    }
}

#[derive(Debug)]
pub struct RealtimeClientBuilder {
    headers: HeaderMap,
    params: Option<HashMap<String, String>>,
    heartbeat_interval: Duration,
    client_ref: Option<String>,
    encode: Option<String>, // placeholder
    decode: Option<String>, // placeholder
    reconnect_interval: ReconnectFn,
    reconnect_max_attempts: usize,
    connection_timeout: Duration,
    auth_url: Option<String>,
    endpoint: String,
    access_token: String,
    max_events_per_second: usize,
}

/// Builds by value / Consuming builder. Cos there's a dyn Fn() in there, i can't clone it
/// inb4 skill issue
impl RealtimeClientBuilder {
    pub fn new(endpoint: String, access_token: String) -> Self {
        let mut headers = HeaderMap::new();
        headers.insert("X-Client-Info", "realtime-rs/0.1.0".parse().unwrap());

        Self {
            headers,
            params: Default::default(),
            heartbeat_interval: Duration::from_secs(29),
            client_ref: Default::default(),
            encode: Default::default(),
            decode: Default::default(),
            reconnect_interval: ReconnectFn(Box::new(backoff)),
            reconnect_max_attempts: usize::MAX,
            connection_timeout: Duration::from_secs(10),
            auth_url: Default::default(),
            endpoint,
            access_token,
            max_events_per_second: 10,
        }
    }

    pub fn set_headers(mut self, set_headers: HeaderMap) -> Self {
        let mut headers = HeaderMap::new();
        headers.insert("X-Client-Info", "realtime-rs/0.1.0".parse().unwrap());
        headers.extend(set_headers);

        self.headers = headers;

        self
    }

    pub fn add_headers(mut self, headers: HeaderMap) -> Self {
        self.headers.extend(headers);
        self
    }

    pub fn params(mut self, params: HashMap<String, String>) -> Self {
        self.params = Some(params);
        self
    }

    pub fn heartbeat_interval(mut self, heartbeat_interval: Duration) -> Self {
        self.heartbeat_interval = heartbeat_interval;
        self
    }

    pub fn client_ref(mut self, client_ref: String) -> Self {
        self.client_ref = Some(client_ref);
        self
    }

    pub fn encode(mut self, encode: String) -> Self {
        self.encode = Some(encode);
        self
    }

    pub fn decode(mut self, decode: String) -> Self {
        self.decode = Some(decode);
        self
    }

    pub fn reconnect_interval(mut self, reconnect_interval: ReconnectFn) -> Self {
        // TODO minimum interval to prevent 10000000 requests in seconds
        // then again it takes a bit of work to make that mistake?
        self.reconnect_interval = reconnect_interval;
        self
    }

    pub fn reconnect_max_attempts(mut self, max_attempts: usize) -> Self {
        self.reconnect_max_attempts = max_attempts;
        self
    }

    pub fn connection_timeout(mut self, timeout: Duration) -> Self {
        // 1 sec min timeout
        // TODO document the minimum timeout
        let timeout = if timeout < Duration::from_secs(1) {
            Duration::from_secs(1)
        } else {
            timeout
        };

        self.connection_timeout = timeout;
        self
    }

    pub fn auth_url(mut self, auth_url: String) -> Self {
        self.auth_url = Some(auth_url);
        self
    }

    pub fn max_events_per_second(mut self, count: usize) -> Self {
        self.max_events_per_second = count;
        self
    }

    pub fn build(self) -> RealtimeClient {
        RealtimeClient {
            headers: self.headers,
            params: self.params,
            heartbeat_interval: self.heartbeat_interval,
            client_ref: self.client_ref,
            encode: self.encode,
            decode: self.decode,
            reconnect_interval: self.reconnect_interval,
            reconnect_max_attempts: self.reconnect_max_attempts,
            connection_timeout: self.connection_timeout,
            auth_url: self.auth_url,
            endpoint: self.endpoint,
            access_token: self.access_token,
            max_events_per_second: self.max_events_per_second,
            ..Default::default()
        }
    }
}

fn backoff(attempts: usize) -> Duration {
    let times: Vec<u64> = vec![0, 1, 2, 5, 10];

    Duration::from_secs(times[attempts.min(times.len() - 1)])
}
