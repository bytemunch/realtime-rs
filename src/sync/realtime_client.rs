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

use reqwest::header::CONTENT_TYPE;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tungstenite::Message;
use tungstenite::{
    client::{uri_mode, IntoClientRequest},
    client_tls,
    handshake::MidHandshake,
    http::{HeaderMap, HeaderValue, Response, StatusCode, Uri},
    stream::{MaybeTlsStream, Mode, NoDelay},
    ClientHandshake, Error, HandshakeError, WebSocket,
};
use uuid::Uuid;

use crate::message::payload::Payload;
use crate::message::realtime_message::RealtimeMessage;
use crate::sync::realtime_channel::{ChannelCreateError, ChannelState, RealtimeChannel};

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

pub struct RealtimeClientOptions {
    pub headers: Option<HeaderMap>,
    pub params: Option<HashMap<String, String>>,
    pub heartbeat_interval: Duration,
    pub client_ref: Option<String>,
    pub encode: Option<String>,                             // placeholder
    pub decode: Option<String>,                             // placeholder
    pub reconnect_interval: Box<dyn Fn(usize) -> Duration>, // impl debug euuurgh there's got to be a better way
    pub reconnect_max_attempts: usize,
    pub connection_timeout: Duration,
    pub auth_url: Option<String>,
}

impl Debug for RealtimeClientOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TODO debug options")
    }
}

impl Default for RealtimeClientOptions {
    fn default() -> Self {
        let mut headers = HeaderMap::new();
        headers.insert("X-Client-Info", "realtime-rs/0.1.0".parse().unwrap());

        Self {
            headers: Some(headers),
            params: None,
            heartbeat_interval: Duration::from_secs(29),
            client_ref: Default::default(),
            encode: None,
            decode: None,
            reconnect_interval: Box::new(backoff),
            reconnect_max_attempts: usize::MAX,
            connection_timeout: Duration::from_secs(10),
            auth_url: None,
        }
    }
}

pub struct RealtimeClient {
    pub socket: Option<WebSocket<MaybeTlsStream<TcpStream>>>,
    channels: HashMap<Uuid, RealtimeChannel>,
    pub status: ConnectionState,
    endpoint: String,
    pub access_token: String,
    // mpsc
    inbound_channel: (Sender<RealtimeMessage>, Receiver<RealtimeMessage>),
    pub(crate) outbound_channel: (Sender<RealtimeMessage>, Receiver<RealtimeMessage>),
    monitor_channel: (Sender<MonitorSignal>, Receiver<MonitorSignal>),
    middleware: HashMap<Uuid, Box<dyn Fn(RealtimeMessage) -> RealtimeMessage>>,
    /// Options to be used in internal fns
    options: RealtimeClientOptions,
    // timers
    reconnect_now: Option<SystemTime>,
    reconnect_delay: Duration,
    reconnect_attempts: usize,
    heartbeat_now: Option<SystemTime>,
}

impl Default for RealtimeClient {
    fn default() -> Self {
        Self {
            socket: Default::default(),
            channels: Default::default(),
            inbound_channel: mpsc::channel(),
            outbound_channel: mpsc::channel(),
            monitor_channel: mpsc::channel(),
            middleware: Default::default(),
            options: Default::default(),
            status: Default::default(),
            endpoint: Default::default(),
            access_token: Default::default(),
            reconnect_delay: Default::default(),
            reconnect_now: Default::default(),
            reconnect_attempts: Default::default(),
            heartbeat_now: Default::default(),
        }
    }
}

impl Debug for RealtimeClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // TODO this is horrid
        f.write_fmt(format_args!(
            "{:?} {:?} {:?} {:?} {:?} {}",
            self.socket,
            self.channels,
            self.inbound_channel,
            self.outbound_channel,
            self.options,
            "TODO middleware debug fmt"
        ))
    }
}

impl RealtimeClient {
    pub fn new(url: String, anon_key: String) -> RealtimeClient {
        RealtimeClient {
            endpoint: url.parse().unwrap(),
            access_token: anon_key,
            inbound_channel: mpsc::channel(),
            outbound_channel: mpsc::channel(),
            monitor_channel: mpsc::channel(),
            ..Default::default()
        }
    }

    pub fn set_options(&mut self, options: RealtimeClientOptions) -> &mut RealtimeClient {
        self.options = options;
        self
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

        let Ok(mut request) = uri.clone().into_client_request() else {
            return Err(ConnectError::BadUri);
        };

        let headers = request.headers_mut();

        let auth: HeaderValue = format!("Bearer {}", self.access_token)
            .parse()
            .expect("malformed access token?");
        headers.insert("Authorization", auth);

        // unwrap: shouldn't fail
        let xci: HeaderValue = format!("realtime-rs/0.1.0").parse().unwrap();
        headers.insert("X-Client-Info", xci);

        println!("Connecting... Req: {:?}\n", request);

        let uri = request.uri();

        let Ok(mode) = uri_mode(uri) else {
            return Err(ConnectError::BadUri);
        };

        let Some(host) = request.uri().host() else {
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
            self.options.connection_timeout,
        ) {
            Ok(stream) => {
                self.reconnect_attempts = 0;
                stream
            }
            Err(_e) => {
                if self.reconnect_attempts < self.options.reconnect_max_attempts {
                    self.reconnect_attempts += 1;
                    let backoff = self.options.reconnect_interval.as_ref();
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

        let Ok(()) = stream.set_nonblocking(true) else {
            return Err(ConnectError::NonblockingError);
        };

        let conn: Result<
            (
                WebSocket<MaybeTlsStream<TcpStream>>,
                Response<Option<Vec<u8>>>,
            ),
            Error,
        > = match client_tls(request, stream) {
            Ok(stream) => {
                self.reconnect_attempts = 0;
                Ok(stream)
            }
            Err(err) => match err {
                HandshakeError::Failure(_err) => {
                    // TODO DRY break reconnect attempts code into own fn
                    if self.reconnect_attempts < self.options.reconnect_max_attempts {
                        self.reconnect_attempts += 1;
                        let backoff = self.options.reconnect_interval.as_ref();
                        sleep(backoff(self.reconnect_attempts));
                        return self.connect();
                    }

                    return Err(ConnectError::HandshakeError);
                }
                HandshakeError::Interrupted(mid_hs) => match self.retry_handshake(mid_hs) {
                    Ok(stream) => Ok(stream),
                    Err(_err) => {
                        if self.reconnect_attempts < self.options.reconnect_max_attempts {
                            self.reconnect_attempts += 1;
                            let backoff = self.options.reconnect_interval.as_ref();
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

        println!("Socket ready");

        self.status = ConnectionState::Open;

        Ok(self)
    }

    fn retry_handshake(
        &mut self,
        mid_hs: MidHandshake<ClientHandshake<MaybeTlsStream<TcpStream>>>,
    ) -> Result<
        (
            WebSocket<MaybeTlsStream<TcpStream>>,
            tungstenite::http::Response<Option<Vec<u8>>>,
        ),
        SocketError,
    > {
        match mid_hs.handshake() {
            Ok(stream) => {
                return Ok(stream);
            }
            Err(e) => match e {
                HandshakeError::Interrupted(mid_hs) => {
                    // TODO sleeping main thread bad
                    if self.reconnect_attempts < self.options.reconnect_max_attempts {
                        self.reconnect_attempts += 1;
                        let backoff = self.options.reconnect_interval.as_ref();
                        sleep(backoff(self.reconnect_attempts));
                        return self.retry_handshake(mid_hs);
                    }

                    return Err(SocketError::TooManyRetries);
                }
                HandshakeError::Failure(_err) => {
                    // TODO pass error data
                    return Err(SocketError::HandshakeError);
                }
            },
        }
    }

    fn run_monitor(&mut self) -> Result<(), MonitorError> {
        if self.reconnect_now.is_none() {
            self.reconnect_now = Some(SystemTime::now());

            self.reconnect_delay =
                self.options.reconnect_interval.as_ref()(self.reconnect_attempts);
        }

        match self.monitor_channel.1.try_recv() {
            Ok(signal) => match signal {
                MonitorSignal::Reconnect => {
                    if self.status == ConnectionState::Open
                        || self.status == ConnectionState::Reconnecting
                        || SystemTime::now() < self.reconnect_now.unwrap() + self.reconnect_delay
                    {
                        return Err(MonitorError::WouldBlock);
                    }

                    if self.reconnect_attempts >= self.options.reconnect_max_attempts {
                        return Err(MonitorError::MaxReconnects);
                    }

                    self.status = ConnectionState::Reconnecting;
                    self.reconnect_attempts += 1;
                    self.reconnect_now.take();

                    match self.connect() {
                        Ok(_) => {
                            for (_id, channel) in &mut self.channels {
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

        if self.heartbeat_now.unwrap() + self.options.heartbeat_interval > SystemTime::now() {
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

                    let _ = self.inbound_channel.0.send(message);
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
                let _ = self.monitor_channel.0.send(MonitorSignal::Reconnect);
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

        // Send to server
        // TODO should drain outbound_channel
        let message = self.outbound_channel.1.try_recv();

        match message {
            Ok(message) => {
                let raw_message = serde_json::to_string(&message);
                println!("[SEND] {:?}", raw_message);
                let _ = socket.send(message.into());
                Ok(())
            }
            Err(TryRecvError::Empty) => {
                // do nothing
                Ok(())
            }
            Err(e) => {
                println!("outbound error: {:?}", e);
                self.status = ConnectionState::Reconnect;
                let _ = self.monitor_channel.0.send(MonitorSignal::Reconnect);
                Err(SocketError::WouldBlock)
            }
        }
    }

    fn reconnect(&mut self) {
        self.status = ConnectionState::Reconnect;
        let _ = self.monitor_channel.0.send(MonitorSignal::Reconnect);
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
        self.outbound_channel.0.send(msg)
    }

    pub fn channel(&mut self, topic: String) -> Result<&mut RealtimeChannel, ChannelCreateError> {
        let topic = format!("realtime:{}", topic);

        let new_channel = RealtimeChannel::new(self, topic.clone());

        match new_channel {
            Ok(channel) => {
                let id = channel.id.clone();

                self.channels.insert(id, channel);

                Ok(self.channels.get_mut(&id).unwrap())
            }
            Err(e) => Err(e),
        }
    }

    pub fn get_channel(&mut self, channel_id: Uuid) -> Option<&mut RealtimeChannel> {
        self.channels.get_mut(&channel_id)
    }

    pub fn get_channels(&self) -> &HashMap<Uuid, RealtimeChannel> {
        &self.channels
    }

    pub fn remove_channel(&mut self, channel_id: Uuid) -> Option<RealtimeChannel> {
        if let Some(mut channel) = self.channels.remove(&channel_id) {
            let _ = channel.unsubscribe();

            if self.channels.len() == 0 {
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

            for (_id, channel) in &mut self.channels {
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

    pub fn block_until_subscribed(&mut self, channel_id: Uuid) -> Result<Uuid, ()> {
        // TODO work WITH the borrow checker, not against it. Probably some combination of ref mut
        let channel = self.channels.get_mut(&channel_id).unwrap();

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
                ChannelState::Closed => return Err(()),
                _ => {}
            }
        }

        Ok(channel_id)
    }

    pub fn sign_in_with_email_password(&mut self, email: String, password: String) {
        let client = reqwest::blocking::Client::new(); //TODO one reqwest client per realtime client
        let url = self
            .options
            .auth_url
            .clone()
            .unwrap_or(self.endpoint.clone());

        let res = client
            .post(format!("{}/token?grant_type=password", url))
            .header(CONTENT_TYPE, "application/json")
            .body(json!({"email": email, "password": password}).to_string())
            .send();

        if let Ok(res) = res {
            let res: AuthResponse = res.json().unwrap();

            self.set_auth(res.access_token);
        }
    }

    pub fn set_auth(&mut self, access_token: String) {
        self.access_token = access_token.clone();

        for (_id, channel) in &mut self.channels {
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
        for (_uuid, middleware) in &self.middleware {
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
            let _ = self.monitor_channel.0.send(MonitorSignal::Reconnect);
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

        for (_id, channel) in &mut self.channels {
            if let Err(_e) = channel.drain_queue() {
                // all errors here are wouldblock, i think
                //return Err(NextMessageError::NoChannel);
            }
        }

        match self.write_socket() {
            Ok(()) => {}
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

        let message = self.inbound_channel.1.try_recv();

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
                        ids.push(id.clone());
                    }
                }

                Ok(ids)
            }
            Err(TryRecvError::Empty) => Err(NextMessageError::WouldBlock),
            Err(e) => Err(NextMessageError::TryRecvError(e)),
        }
    }
}

fn backoff(attempts: usize) -> Duration {
    let times: Vec<u64> = vec![0, 1, 2, 5, 10];

    Duration::from_secs(times[attempts.min(times.len() - 1)])
}
