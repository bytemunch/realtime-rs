#![allow(dead_code)]

use std::fmt::{Debug, Display};
use std::thread::sleep;
use std::time::SystemTime;
use std::{
    collections::HashMap,
    io,
    net::{SocketAddr, TcpStream, ToSocketAddrs},
    sync::mpsc::{self, Receiver, Sender, TryRecvError},
    time::Duration,
};

use tungstenite::Message;
use tungstenite::{
    client::{uri_mode, IntoClientRequest},
    client_tls,
    error::UrlError,
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
    SocketError(ConnectionError),
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
pub enum ConnectionError {
    NoSocket,
    NoRead,
    NoWrite,
    Disconnected,
}

#[derive(Debug, PartialEq)]
pub enum MonitorSignal {
    Reconnect,
}

pub struct RealtimeClientOptions {
    pub headers: Option<HeaderMap>,
    pub params: Option<HashMap<String, String>>,
    pub timeout: Duration, // TODO placeholder
    pub heartbeat_interval: Duration,
    pub client_ref: Option<String>,
    pub encode: Option<String>,                             // placeholder
    pub decode: Option<String>,                             // placeholder
    pub reconnect_interval: Box<dyn Fn(usize) -> Duration>, // impl debug euuurgh there's got to be a better way
    pub reconnect_max_attempts: usize,
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
            timeout: Duration::from_secs(10),
            heartbeat_interval: Duration::from_secs(29),
            client_ref: Default::default(),
            encode: None,
            decode: None,
            reconnect_interval: Box::new(backoff),
            reconnect_max_attempts: usize::MAX,
        }
    }
}

pub struct RealtimeClient {
    pub socket: Option<WebSocket<MaybeTlsStream<TcpStream>>>,
    channels: HashMap<Uuid, RealtimeChannel>,
    pub status: ConnectionState,
    endpoint: String,
    access_token: String,
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

    pub fn connect(&mut self) -> Result<&mut RealtimeClient, Box<dyn std::error::Error>> {
        // TODO error enum for this fn
        let uri: Uri = format!(
            "{}/realtime/v1/websocket?apikey={}&vsn=1.0.0",
            self.endpoint, self.access_token
        )
        .parse()?;

        let mut request = uri.clone().into_client_request()?;
        let headers = request.headers_mut();

        let auth: HeaderValue = format!("Bearer {}", self.access_token).parse()?;
        headers.insert("Authorization", auth);

        let xci: HeaderValue = format!("realtime-rs/0.1.0").parse()?;
        headers.insert("X-Client-Info", xci);

        println!("Connecting... Req: {:?}\n", request);

        let uri = request.uri();
        let mode = uri_mode(uri)?;
        let host = request
            .uri()
            .host()
            .ok_or(Error::Url(UrlError::NoHostName))?;

        let host = if host.starts_with('[') {
            &host[1..host.len() - 1]
        } else {
            host
        };

        let port = uri.port_u16().unwrap_or(match mode {
            Mode::Plain => 80,
            Mode::Tls => 443,
        });

        let addrs = (host, port).to_socket_addrs()?;

        fn get_stream(addrs: &[SocketAddr], uri: &Uri) -> Result<TcpStream, &'static str> {
            for addr in addrs {
                println!("Trying to contact {} at {}...", uri, addr);
                if let Ok(stream) = TcpStream::connect(addr) {
                    return Ok(stream);
                }
            }

            Err("Stream no worky")
        }

        let mut stream = match get_stream(addrs.as_slice(), uri) {
            Ok(stream) => stream,
            Err(e) => return Err(e.into()),
        };

        NoDelay::set_nodelay(&mut stream, true)?;

        stream.set_nonblocking(true)?;

        // TODO alias these types if they keep coming up
        fn retry_handshake(
            mid_hs: MidHandshake<ClientHandshake<MaybeTlsStream<TcpStream>>>,
            attempts: usize,
            reconnect_fn: &Box<dyn Fn(usize) -> Duration>,
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
                Err(e) => match e {
                    HandshakeError::Interrupted(mid_hs) => {
                        // TODO sleeping main thread bad
                        sleep(reconnect_fn(attempts));

                        return retry_handshake(mid_hs, attempts + 1, reconnect_fn);
                    }
                    HandshakeError::Failure(err) => {
                        return Err(err);
                    }
                },
            }
        }

        fn get_connection<R>(
            request: R,
            stream: TcpStream,
            backoff_fn: &Box<dyn Fn(usize) -> Duration>,
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
                    HandshakeError::Interrupted(mid_hs) => {
                        return retry_handshake(mid_hs, 0, backoff_fn)
                    }
                },
            }
        }

        let conn = get_connection(request, stream, &self.options.reconnect_interval)?;

        let (socket, res) = conn;

        if res.status() != StatusCode::SWITCHING_PROTOCOLS {
            return Err("101 code not recieved.".into());
        }

        self.socket = Some(socket);

        println!("Socket ready");

        self.status = ConnectionState::Open;
        self.reconnect_attempts = 0;

        Ok(self)
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
                    if self.status == ConnectionState::Open {
                        return Ok(()); // TODO wouldblock
                    }

                    if self.status == ConnectionState::Reconnecting
                        || SystemTime::now() < self.reconnect_now.unwrap() + self.reconnect_delay
                    {
                        return Ok(()); // TODO wouldblock
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

        self.send(RealtimeMessage::heartbeat());
    }

    fn read_socket(&mut self) -> Result<(), ()> {
        let Some(ref mut socket) = self.socket else {
            return Err(());
        };

        if !socket.can_read() {
            self.status = ConnectionState::Reconnect;
            let _ = self.monitor_channel.0.send(MonitorSignal::Reconnect);
            return Err(());
        }

        match socket.read() {
            Ok(raw_message) => match raw_message {
                Message::Text(string_message) => {
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
                    // TODO disconnect
                    Err(())
                }
                _ => {
                    // do nothing on ping, pong, binary messages
                    Ok(())
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
                Err(())
            }
        }
    }

    fn write_socket(&mut self) -> Result<(), ()> {
        let Some(ref mut socket) = self.socket else {
            return Err(());
        };

        if !socket.can_write() {
            self.status = ConnectionState::Reconnect;
            let _ = self.monitor_channel.0.send(MonitorSignal::Reconnect);
            return Err(());
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
                Err(())
            }
        }
    }

    pub fn disconnect(&mut self) {
        // TODO i don't like this circular stuff bouncing between disconnect() and
        // remove_all_channels()
        //
        // might just duplicate the channel remove code
        self.remove_all_channels();

        if self.status == ConnectionState::Closed {
            return;
        }

        let Some(ref mut socket) = self.socket else {
            self.status = ConnectionState::Closed;
            println!("Already disconnected. {:?}", self.status);
            return;
        };

        let _ = socket.close(None);
        self.status = ConnectionState::Closed;
        println!("Client disconnected. {:?}", self.status);
    }

    pub fn send(&mut self, msg: RealtimeMessage) {
        let _ = self.outbound_channel.0.send(msg);
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

    pub fn get_channel(&self, channel_id: Uuid) -> Option<&RealtimeChannel> {
        self.channels.get(&channel_id)
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

    pub fn set_auth(&mut self, _access_token: String) {
        // TODO
    }

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

    /// Polls next message from socket. Returns the ids of the channels that has been forwarded
    /// the message, or a TryRecvError. Can return WouldBlock
    pub fn next_message(&mut self) -> Result<Vec<Uuid>, NextMessageError> {
        if self.status == ConnectionState::Closed {
            return Err(NextMessageError::ClientClosed);
        }

        match self.run_monitor() {
            Ok(_) => {}
            Err(MonitorError::WouldBlock) => {}
            Err(MonitorError::MaxReconnects) => {
                // TODO ummmmm like kill the reconnect attempts now
                self.disconnect();
                return Err(NextMessageError::MonitorError(MonitorError::MaxReconnects));
            }
            Err(e) => {
                return Err(NextMessageError::MonitorError(e));
            }
        }

        self.run_heartbeat();
        let _ = self.read_socket();
        let _ = self.write_socket();

        if self.status == ConnectionState::Reconnect {
            self.status = ConnectionState::Reconnect;
            let _ = self.monitor_channel.0.send(MonitorSignal::Reconnect);
            return Err(NextMessageError::SocketError(ConnectionError::Disconnected));
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
