#![allow(dead_code)]

use std::fmt::Debug;
use std::sync::MutexGuard;
use std::thread::JoinHandle;
use std::{
    collections::HashMap,
    env, io,
    net::{SocketAddr, TcpStream, ToSocketAddrs},
    sync::{
        mpsc::{self, Receiver, Sender, TryRecvError},
        Arc, Mutex,
    },
    thread::{self, sleep},
    time::Duration,
};

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
use crate::realtime_channel::{ChannelCreateError, ChannelState, RealtimeChannel};

#[derive(PartialEq, Debug)]
pub enum ConnectionState {
    Connecting,
    Open,
    Closing,
    Closed,
}

#[derive(PartialEq, Debug)]
pub enum NextMessageError {
    WouldBlock,
    TryRecvError(TryRecvError),
    NoChannel,
    ChannelClosed,
    SocketError(ConnectionError),
}

#[derive(Debug, PartialEq)]
pub enum ConnectionError {
    NoSocket,
    NoRead,
    NoWrite,
    Disconnected,
    MutexError, // TODO return the poisonerror here, lifetime tings
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
            timeout: Duration::from_secs(10),
            heartbeat_interval: Duration::from_secs(29),
            client_ref: 0,
            encode: None,
            decode: None,
            reconnect_interval: None,
        }
    }
}

pub struct RealtimeClient {
    pub socket: Arc<Mutex<Option<WebSocket<MaybeTlsStream<TcpStream>>>>>,
    channels: HashMap<Uuid, RealtimeChannel>,
    inbound_channel: Option<(Sender<RealtimeMessage>, Receiver<RealtimeMessage>)>,
    pub(crate) outbound_tx: Option<Sender<RealtimeMessage>>,
    middleware: HashMap<Uuid, Box<dyn Fn(RealtimeMessage) -> RealtimeMessage>>,
    /// Options to be used in internal fns
    options: RealtimeClientOptions,
    status: ConnectionState,
    endpoint: String,
    access_token: String,
    // threads
    heartbeat_thread: Option<JoinHandle<()>>,
    socket_rw_thread: Option<JoinHandle<()>>,
    monitor_thread: Option<JoinHandle<()>>,
}

impl Debug for RealtimeClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(format_args!(
            "{:?} {:?} {:?} {:?} {:?} {}",
            self.socket,
            self.channels,
            self.inbound_channel,
            self.outbound_tx,
            self.options,
            "TODO middleware debug fmt"
        ))
    }
}

impl RealtimeClient {
    pub fn new(url: String, anon_key: String) -> RealtimeClient {
        RealtimeClient {
            socket: Arc::new(Mutex::new(None)),
            channels: HashMap::new(),
            inbound_channel: None,
            outbound_tx: None,
            middleware: HashMap::new(),
            options: RealtimeClientOptions::default(),
            status: ConnectionState::Closed,
            endpoint: url.parse().unwrap(),
            access_token: anon_key,
            heartbeat_thread: None,
            socket_rw_thread: None,
            monitor_thread: None,
        }
    }

    pub fn connect(&mut self) -> &mut RealtimeClient {
        let uri: Uri = format!(
            "{}/realtime/v1/websocket?apikey={}&vsn=1.0.0",
            self.endpoint, self.access_token
        )
        .parse()
        .expect("Malformed URI");

        let mut request = uri.clone().into_client_request().unwrap();
        let headers = request.headers_mut();

        let auth: HeaderValue = format!("Bearer {}", self.access_token).parse().unwrap();
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
                            // TODO sleep for a bit here?
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

        let socket = Arc::new(Mutex::new(Some(socket)));
        self.socket = socket;

        self.start_socket_rw_thread();

        self.start_heartbeat_thread();

        println!("Socket ready");

        self.status = ConnectionState::Open;

        self
    }

    fn start_heartbeat_thread(&mut self) {
        let heartbeat_socket = self.socket.clone();

        let heartbeat_interval = self.options.heartbeat_interval;

        self.heartbeat_thread = Some(thread::spawn(move || loop {
            // sleep thread
            sleep(heartbeat_interval);

            match heartbeat_socket.lock() {
                Ok(socket_guard) => {
                    let Ok(mut socket_guard) = check_connection_static(socket_guard) else {
                        continue;
                    };

                    let Some(ref mut socket) = *socket_guard else {
                        continue;
                    };

                    let _ = socket.send(RealtimeMessage::heartbeat().into());

                    drop(socket_guard);
                }
                Err(err) => {
                    println!("Heartbeat thread: mutex hard. {:?}", err);
                }
            };
        }));
    }

    fn start_socket_rw_thread(&mut self) {
        let outbound_channel = mpsc::channel::<RealtimeMessage>();
        let outbound_tx = outbound_channel.0;
        let outbound_rx = outbound_channel.1;

        let inbound_channel = mpsc::channel::<RealtimeMessage>();

        let inbound_tx = inbound_channel.0.clone();

        let loop_socket = self.socket.clone();

        self.socket_rw_thread = Some(thread::spawn(move || loop {
            // TODO nested way too much for my liking
            match loop_socket.lock() {
                Ok(socket_guard) => {
                    let Ok(mut socket_guard) = check_connection_static(socket_guard) else {
                        continue;
                    };

                    let Some(ref mut socket) = *socket_guard else {
                        continue;
                    };
                    // Send to server
                    let message = outbound_rx.try_recv();

                    match message {
                        Ok(message) => {
                            let raw_message = serde_json::to_string(&message);
                            println!("[SEND] {:?}", raw_message);
                            let _ = socket.send(message.into());
                        }
                        Err(TryRecvError::Empty) => { // do nothing
                        }
                        Err(e) => {
                            println!("outbound error: {:?}", e);
                        }
                    }

                    // Recieve from server
                    match socket.read() {
                        Ok(raw_message) => {
                            let msg = raw_message.to_text().unwrap();

                            let message: RealtimeMessage =
                                serde_json::from_str(msg).expect("Deserialization error: ");

                            println!("[RECV] {:?}", message);

                            // TODO error handling; match on all msg values
                            match message.payload {
                                Payload::Empty {} => {
                                    println!(
                                        "Possibly malformed payload: {:?}",
                                        raw_message.to_text().unwrap()
                                    )
                                }
                                _ => {}
                            }
                            let _ = inbound_tx.send(message);
                        }
                        Err(Error::Io(err)) if err.kind() == io::ErrorKind::WouldBlock => {
                            // do nothing here :)
                        }
                        Err(err) => {
                            println!("Socket read error: {:?}", err);
                        }
                    }

                    drop(socket_guard);
                }
                Err(err) => {
                    println!("Socket thread: mutex hard. {:?}", err);
                }
            }
        }));

        self.inbound_channel = Some(inbound_channel);
        self.outbound_tx = Some(outbound_tx);
    }

    fn kill_all_threads(&mut self) {
        if let Some(thread) = self.heartbeat_thread.take() {
            let _ = thread.join();
        }

        if let Some(thread) = self.socket_rw_thread.take() {
            let _ = thread.join();
        }

        if let Some(thread) = self.monitor_thread.take() {
            let _ = thread.join();
        }
    }

    pub fn disconnect(&mut self) {
        // TODO i don't like this circular stuff bouncing between disconnect() and
        // remove_all_channels()
        //
        // might just duplicate the channel remove code
        self.remove_all_channels();

        if self.status == ConnectionState::Closed {
            self.kill_all_threads();
            return;
        }

        match self.socket.lock() {
            Ok(mut socket_guard) => {
                let Some(ref mut socket) = *socket_guard else {
                    self.status = ConnectionState::Closed;
                    println!("Already disconnected. {:?}", self.status);
                    return;
                };
                let _ = socket.close(None);
                drop(socket_guard);
                self.status = ConnectionState::Closed;
                println!("Client disconnected. {:?}", self.status);
            }
            Err(e) => {
                println!("Disconnect: mutex hard. {:?}", e);
            }
        }
    }

    pub fn send(&mut self, msg: RealtimeMessage) {
        match self.socket.lock() {
            Ok(mut socket_guard) => {
                let Some(ref mut socket) = *socket_guard else {
                    return;
                };
                println!("Sending: {:?}", msg);
                let _ = socket.send(msg.into());
                drop(socket_guard);
            }
            Err(err) => {
                println!("Send: mutex hard. {:?}", err);
            }
        };
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

        // wait until inbound_rx is drained
        loop {
            let recv = self.next_message();

            if Err(NextMessageError::WouldBlock) == recv {
                break;
            }
        }

        self.status = ConnectionState::Closing;

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

        self.disconnect(); // No need to stay connected with no channels
    }

    pub fn set_auth(&mut self, _access_token: String) {
        // TODO
    }

    pub fn add_middleware(
        &mut self,
        middleware: Box<dyn Fn(RealtimeMessage) -> RealtimeMessage>,
    ) -> Uuid {
        // TODO user defined middleware ordering
        let uuid = Uuid::new_v4();
        self.middleware.insert(uuid, middleware);
        uuid
    }

    pub fn run_middleware(&self, mut message: RealtimeMessage) -> RealtimeMessage {
        for (_uuid, middleware) in &self.middleware {
            message = middleware(message)
        }
        message
    }

    pub fn remove_middleware(
        &mut self,
        uuid: Uuid,
    ) -> Option<Box<dyn Fn(RealtimeMessage) -> RealtimeMessage>> {
        self.middleware.remove(&uuid)
    }

    fn check_connection(&mut self) -> Result<(), ConnectionError> {
        let Ok(socket) = self.socket.lock() else {
            // TODO reconnect
            return Err(ConnectionError::MutexError);
        };

        match check_connection_static(socket) {
            Ok(socket) => {
                drop(socket);
                Ok(())
            }
            Err(e) => Err(e),
        }
    }

    /// Polls next message from socket. Returns the topic of the channel that has been forwarded
    /// the message, or a TryRecvError. Can return WouldBlock
    pub fn next_message(&mut self) -> Result<Vec<Uuid>, NextMessageError> {
        match self.check_connection() {
            Ok(_) => {}
            Err(e) => return Err(NextMessageError::SocketError(e)),
        }

        let Some(inbound_channel) = &self.inbound_channel else {
            return Err(NextMessageError::ChannelClosed);
        };

        let message = inbound_channel.1.try_recv();

        match message {
            Ok(mut message) => {
                let mut ids = vec![];
                // TODO filter system messages and the like

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

    /// Blocking listener loop, use if all business logic is in callbacks
    pub fn listen(&mut self) {
        let Some(inbound_channel) = &self.inbound_channel else {
            // TODO error handling
            return;
        };

        for message in &inbound_channel.1 {
            // TODO filter system messages and the like
            // Send message to channel
            for (_id, channel) in &mut self.channels {
                if channel.topic == message.topic {
                    channel.recieve(message.clone());
                }
            }
        }
    }
}

fn check_connection_static<'a>(
    socket_guard: MutexGuard<'a, Option<WebSocket<MaybeTlsStream<TcpStream>>>>,
) -> Result<MutexGuard<'a, Option<WebSocket<MaybeTlsStream<TcpStream>>>>, ConnectionError> {
    let Some(ref inner_socket) = *socket_guard else {
        return Err(ConnectionError::NoSocket);
    };

    if !inner_socket.can_read() {
        drop(socket_guard);
        return Err(ConnectionError::NoRead);
    }

    if !inner_socket.can_write() {
        drop(socket_guard);
        return Err(ConnectionError::NoWrite);
    }

    Ok(socket_guard)
}
