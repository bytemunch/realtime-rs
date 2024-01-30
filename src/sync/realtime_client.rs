use std::fmt::{Debug, Display};
use std::sync::Arc;
use std::time::SystemTime;
use std::{
    collections::HashMap,
    sync::mpsc::{self, Receiver, Sender, TryRecvError},
    time::Duration,
};

use tokio::io::AsyncReadExt;
use tokio::sync::Mutex;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::{HeaderMap, HeaderValue, Uri};
use tokio_tungstenite::tungstenite::Message;

use uuid::Uuid;

use futures_util::{future, pin_mut, StreamExt};

use crate::message::RealtimeMessage;
use crate::sync::realtime_channel::RealtimeChannel;
use crate::DEBUG;

use super::realtime_channel::RealtimeChannelBuilder;
// Our helper method which will read data from stdin and send it along the
// sender provided.
async fn read_stdin(tx: futures_channel::mpsc::UnboundedSender<Message>) {
    let mut stdin = tokio::io::stdin();
    loop {
        let mut buf = vec![0; 1024];
        let n = match stdin.read(&mut buf).await {
            Err(_) | Ok(0) => break,
            Ok(n) => n,
        };
        buf.truncate(n);
        tx.unbounded_send(Message::binary(buf)).unwrap();
    }
}
/// Error type for [RealtimeClient::sign_in_with_email_password()]
#[derive(PartialEq, Debug)]
pub enum AuthError {
    BadLogin,
    MalformedData,
}

/// Connection state of [RealtimeClient]
#[derive(PartialEq, Debug, Default, Clone, Copy)]
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

/// Error returned by [RealtimeClient::next_message()].
/// Can be WouldBlock
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

/// Error returned by the internal monitor of [RealtimeClient]
#[derive(PartialEq, Debug)]
pub enum MonitorError {
    ReconnectError,
    MaxReconnects,
    WouldBlock,
    Disconnected,
}

/// Error type for internal socket related errors in [RealtimeClient]
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

/// Error returned by [RealtimeClient::connect()]
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

/// Newtype for [`mpsc::channel<RealtimeMessage>`]
pub(crate) struct MessageChannel((Sender<RealtimeMessage>, Receiver<RealtimeMessage>));

impl Default for MessageChannel {
    fn default() -> Self {
        Self(mpsc::channel())
    }
}

struct MonitorChannel((Sender<MonitorSignal>, Receiver<MonitorSignal>));

impl Default for MonitorChannel {
    fn default() -> Self {
        Self(mpsc::channel())
    }
}

#[derive(Debug, PartialEq)]
enum MonitorSignal {
    Reconnect,
}

/// Synchronous websocket client that interfaces with Supabase Realtime
#[derive(Default)]
pub struct RealtimeClient {
    pub(crate) access_token: String,
    status: ConnectionState,
    ws_tx: Option<futures_channel::mpsc::UnboundedSender<Message>>,
    ws_rx: Option<futures_channel::mpsc::UnboundedReceiver<Message>>,
    channels: HashMap<Uuid, RealtimeChannel>,
    channel_tx_list: Arc<Mutex<Vec<(String, futures_channel::mpsc::UnboundedSender<Message>)>>>,
    messages_this_second: Vec<SystemTime>,
    next_ref: Uuid,
    // mpsc
    pub(crate) outbound_channel: MessageChannel,
    inbound_channel: MessageChannel,
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
    encode: Option<Box<dyn Fn(RealtimeMessage) -> RealtimeMessage>>,
    decode: Option<Box<dyn Fn(RealtimeMessage) -> RealtimeMessage>>,
    reconnect_interval: ReconnectFn,
    reconnect_max_attempts: usize,
    connection_timeout: Duration,
    endpoint: String,
    max_events_per_second: usize,
}

impl Debug for RealtimeClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // TODO this is horrid
        f.write_fmt(format_args!("todo"))
    }
}

impl RealtimeClient {
    /// Returns a new [RealtimeClientBuilder] with provided `endpoint` and `access_token`
    pub fn builder(
        endpoint: impl Into<String>,
        access_token: impl Into<String>,
    ) -> RealtimeClientBuilder {
        RealtimeClientBuilder::new(endpoint, access_token)
    }

    /// Returns this client's [ConnectionState]
    pub fn get_status(&self) -> ConnectionState {
        self.status
    }

    /// Returns a new [RealtimeChannelBuilder] instantiated with the provided `topic`
    /// TODO CODE
    /// ```
    pub fn channel(&mut self, topic: impl Into<String>) -> RealtimeChannelBuilder {
        RealtimeChannelBuilder::new(self).topic(topic)
    }

    /// Attempt to create a websocket connection with the server
    /// TODO CODE
    /// ```
    pub async fn connect(&mut self) -> Result<&mut RealtimeClient, ConnectError> {
        let uri: Uri = match format!(
            "{}/realtime/v1/websocket?apikey={}&vsn=1.0.0",
            self.endpoint, self.access_token
        )
        .parse()
        {
            Ok(uri) => uri,
            Err(_e) => return Err(ConnectError::BadUri),
        };

        let ws_scheme = match uri.scheme_str() {
            Some(scheme) => {
                if scheme == "http" {
                    "ws"
                } else {
                    "wss"
                }
            }
            None => "wss",
        };

        let mut add_params = String::new();
        if let Some(params) = &self.params {
            for (field, value) in params {
                add_params = format!("{add_params}&{field}={value}");
            }
        }

        let mut p_q = uri.path_and_query().unwrap().to_string();

        if !add_params.is_empty() {
            p_q = format!("{p_q}{add_params}");
        }

        let uri = Uri::builder()
            .scheme(ws_scheme)
            .authority(uri.authority().unwrap().clone())
            .path_and_query(p_q)
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

        let xci: HeaderValue = "realtime-rs/0.1.0".to_string().parse().unwrap();
        headers.insert("X-Client-Info", xci);

        headers.extend(self.headers.clone());

        if DEBUG {
            println!("Connecting... Req: {:?}\n", request);
        }

        let (ws_tx_tx, ws_tx_rx) = futures_channel::mpsc::unbounded();
        let (ws_rx_tx, ws_rx_rx) = futures_channel::mpsc::unbounded();

        let (ws_stream, _) = connect_async(request).await.expect("Failed to connect");
        println!("WebSocket handshake has been successfully completed");

        let (write, mut read) = ws_stream.split();

        let _send_thread = tokio::spawn(async move {
            let sender = ws_tx_rx.map(Ok).forward(write);
            pin_mut!(sender);
            let _ = sender.await;
        });

        let channel_list = self.channel_tx_list.clone();

        let _recieve_thread = tokio::spawn(async move {
            while let Some(msg) = read.next().await {
                if let Ok(msg) = msg {
                    let list = channel_list.lock().await;

                    let list: &Vec<(String, futures_channel::mpsc::UnboundedSender<Message>)> =
                        list.as_ref();

                    // ITS ALIIIIIIVE
                    for (topic, tx) in list {
                        println!("{}, {:?}", topic, tx);
                    }

                    let _ = ws_rx_tx.unbounded_send(msg);

                    // Send msg to each channel's tx
                    //
                    // arc mutex list of (channel filter bits, channel tx)
                }
            }
        });

        self.status = ConnectionState::Open;

        self.ws_tx = Some(ws_tx_tx);
        self.ws_rx = Some(ws_rx_rx);

        let _ = self
            .ws_tx
            .as_mut()
            .unwrap()
            .unbounded_send(RealtimeMessage::heartbeat().into());

        Ok(self)
    }

    pub async fn handle_incoming(&mut self) {
        while let Some(msg) = self.ws_rx.as_mut().unwrap().next().await {
            println!("Got it! {:?}", msg);
        }
    }

    pub fn send(
        &mut self,
        msg: RealtimeMessage,
    ) -> Result<(), futures_channel::mpsc::TrySendError<Message>> {
        self.ws_tx.as_mut().unwrap().unbounded_send(msg.into())
    }

    /// Returns an optional mutable reference to the [RealtimeChannel] with the provided [Uuid].
    /// If `channel_id` is not found returns [None]
    pub fn get_channel_mut(&mut self, channel_id: Uuid) -> Option<&mut RealtimeChannel> {
        self.channels.get_mut(&channel_id)
    }

    /// Returns an optional reference to the [RealtimeChannel] with the provided [Uuid].
    /// If `channel_id` is not found returns [None]
    pub fn get_channel(&self, channel_id: Uuid) -> Option<&RealtimeChannel> {
        self.channels.get(&channel_id)
    }

    /// Returns a reference to this client's HashMap of channels
    pub fn get_channels(&self) -> &HashMap<Uuid, RealtimeChannel> {
        &self.channels
    }

    /// Returns [Some(RealtimeChannel)] if channel was successfully removed, [None] if the channel
    /// was not found.
    pub async fn remove_channel(&mut self, channel_id: Uuid) -> Option<RealtimeChannel> {
        if let Some(mut channel) = self.channels.remove(&channel_id) {
            let _ = channel.unsubscribe();

            return Some(channel);
        }

        None
    }

    /// Use provided JWT to authorize future requests from this client and all channels
    pub fn set_auth(&mut self, access_token: String) {
        self.access_token = access_token.clone();

        for channel in self.channels.values_mut() {
            // TODO single source of data for access token
            let _ = channel.set_auth(access_token.clone()); // TODO error handling
        }
    }

    // TODO look into if middleware is needed?
    /// Add a callback to run mutably on recieved [RealtimeMessage]s before any other registered
    /// callbacks.
    pub fn add_middleware(
        &mut self,
        middleware: Box<dyn Fn(RealtimeMessage) -> RealtimeMessage>,
    ) -> Uuid {
        // TODO user defined middleware ordering
        let uuid = Uuid::new_v4();
        self.middleware.insert(uuid, middleware);
        uuid
    }

    /// Remove middleware by it's [Uuid]
    pub fn remove_middleware(&mut self, uuid: Uuid) -> &mut RealtimeClient {
        self.middleware.remove(&uuid);
        self
    }

    /// The main step function for driving the [RealtimeClient]
    /// TODO code
    /// ```
    pub async fn next_message(&mut self) -> Result<Vec<Uuid>, NextMessageError> {
        match self.status {
            ConnectionState::Closed => {
                return Err(NextMessageError::ClientClosed);
            }
            ConnectionState::Reconnect => {
                let _ = self.monitor_channel.0 .0.send(MonitorSignal::Reconnect);
                return Err(NextMessageError::SocketError(SocketError::Disconnected));
            }
            ConnectionState::Reconnecting => {
                return Err(NextMessageError::WouldBlock);
            }
            ConnectionState::Connecting => {}
            ConnectionState::Closing => {}
            ConnectionState::Open => {}
        }

        match self.run_monitor().await {
            Ok(_) => {}
            Err(MonitorError::WouldBlock) => {}
            Err(e) => {
                return Err(NextMessageError::MonitorError(e));
            }
        }

        self.run_heartbeat();

        match self.inbound_channel.0 .1.try_recv() {
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

    pub(crate) async fn add_channel(&mut self, channel: RealtimeChannel) -> Uuid {
        let id = channel.id;

        let list = self.channel_tx_list.clone();

        let mut list = list.lock().await;

        let list: &mut Vec<(String, futures_channel::mpsc::UnboundedSender<Message>)> =
            list.as_mut();

        list.push((channel.topic.clone(), channel.channel_tx.clone()));

        self.channels.insert(channel.id, channel);

        id
    }

    pub(crate) fn get_channel_tx(&self) -> futures_channel::mpsc::UnboundedSender<Message> {
        self.ws_tx.clone().unwrap()
    }

    fn run_middleware(&self, mut message: RealtimeMessage) -> RealtimeMessage {
        for middleware in self.middleware.values() {
            message = middleware(message)
        }
        message
    }

    async fn run_monitor(&mut self) -> Result<(), MonitorError> {
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

                    match self.connect().await {
                        Ok(_) => {
                            for channel in self.channels.values_mut() {
                                channel.subscribe();
                            }

                            Ok(())
                        }
                        Err(e) => {
                            if DEBUG {
                                println!("reconnect error: {:?}", e);
                            }
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

    fn reconnect(&mut self) {
        self.status = ConnectionState::Reconnect;
        let _ = self.monitor_channel.0 .0.send(MonitorSignal::Reconnect);
    }
}

/// Takes a `Box<dyn Fn(usize) -> Duration>`
/// The provided function should take a count of reconnect attempts and return a [Duration] to wait
/// until the next attempt is made.
pub struct ReconnectFn(pub Box<dyn Fn(usize) -> Duration>);

impl ReconnectFn {
    pub fn new(f: impl Fn(usize) -> Duration + 'static) -> Self {
        Self(Box::new(f))
    }
}

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

/// Builder struct for [RealtimeClient]
/// ```
/// # use std::env;
/// # use realtime_rs::sync::*;
/// # use realtime_rs::message::*;  
/// # use realtime_rs::*;          
/// # fn main() -> Result<(), ()> {
///     let url = "http://127.0.0.1:54321";
///     let anon_key = env::var("LOCAL_ANON_KEY").expect("No anon key!");
///
///     let mut client = RealtimeClientBuilder::new(url, anon_key)
///         .build();
/// #   Ok(())
/// # }
pub struct RealtimeClientBuilder {
    headers: HeaderMap,
    params: Option<HashMap<String, String>>,
    heartbeat_interval: Duration,
    encode: Option<Box<dyn Fn(RealtimeMessage) -> RealtimeMessage>>,
    decode: Option<Box<dyn Fn(RealtimeMessage) -> RealtimeMessage>>,
    reconnect_interval: ReconnectFn,
    reconnect_max_attempts: usize,
    connection_timeout: Duration,
    auth_url: Option<String>,
    endpoint: String,
    access_token: String,
    max_events_per_second: usize,
}

impl RealtimeClientBuilder {
    /// Creates a new [RealtimeClientBuilder]
    pub fn new(endpoint: impl Into<String>, anon_key: impl Into<String>) -> Self {
        let mut headers = HeaderMap::new();
        headers.insert("X-Client-Info", "realtime-rs/0.1.0".parse().unwrap());

        Self {
            headers,
            params: Default::default(),
            heartbeat_interval: Duration::from_secs(29),
            encode: Default::default(),
            decode: Default::default(),
            reconnect_interval: ReconnectFn(Box::new(backoff)),
            reconnect_max_attempts: usize::MAX,
            connection_timeout: Duration::from_secs(10),
            auth_url: Default::default(),
            endpoint: endpoint.into(),
            access_token: anon_key.into(),
            max_events_per_second: 10,
        }
    }

    pub fn access_token(mut self, access_token: impl Into<String>) -> Self {
        self.access_token = access_token.into();

        self
    }

    /// Sets the client headers. Headers always contain "X-Client-Info: realtime-rs/{version}".
    pub fn set_headers(mut self, set_headers: HeaderMap) -> Self {
        let mut headers = HeaderMap::new();
        headers.insert("X-Client-Info", "realtime-rs/0.1.0".parse().unwrap());
        headers.extend(set_headers);

        self.headers = headers;

        self
    }

    /// Merges provided [HeaderMap] with currently held headers
    pub fn add_headers(mut self, headers: HeaderMap) -> Self {
        self.headers.extend(headers);
        self
    }

    /// Set endpoint URL params
    pub fn params(mut self, params: HashMap<String, String>) -> Self {
        self.params = Some(params);
        self
    }

    /// Set [Duration] between heartbeat packets. Default 29 seconds.
    pub fn heartbeat_interval(mut self, heartbeat_interval: Duration) -> Self {
        self.heartbeat_interval = heartbeat_interval;
        self
    }

    /// Set the function to provide time between reconnection attempts
    /// The provided function should take a count of reconnect attempts and return a [Duration] to wait
    /// until the next attempt is made.
    ///
    /// Don't implement an untested timing function here in prod or you might make a few too many
    /// requests.
    ///
    /// Defaults to stepped backoff, as shown below
    /// ```
    /// # use std::env;
    /// # use std::time::Duration;
    /// # use realtime_rs::sync::*;
    /// # use realtime_rs::message::*;  
    /// # use realtime_rs::*;          
    /// # fn main() -> Result<(), ()> {
    ///     fn backoff(attempts: usize) -> Duration {
    ///         let times: Vec<u64> = vec![0, 1, 2, 5, 10];
    ///         Duration::from_secs(times[attempts.min(times.len() - 1)])
    ///     }
    ///
    ///     let url = "http://127.0.0.1:54321";
    ///     let anon_key = env::var("LOCAL_ANON_KEY").expect("No anon key!");
    ///
    ///     let mut client = RealtimeClientBuilder::new(url, anon_key)
    ///         .reconnect_interval(ReconnectFn::new(backoff))
    ///         .build();
    /// #   Ok(())
    /// # }
    ///
    pub fn reconnect_interval(mut self, reconnect_interval: ReconnectFn) -> Self {
        // TODO minimum interval to prevent 10000000 requests in seconds
        // then again it takes a bit of work to make that mistake?
        self.reconnect_interval = reconnect_interval;
        self
    }

    /// Configure the number of recconect attempts to be made before erroring
    pub fn reconnect_max_attempts(mut self, max_attempts: usize) -> Self {
        self.reconnect_max_attempts = max_attempts;
        self
    }

    /// Configure the duration to wait for a connection to succeed.
    /// Default: 10 seconds
    /// Minimum: 1 second
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

    /// Set the base URL for the auth server
    /// In live supabase deployments this is the same as the endpoint URL, and defaults as such.
    /// In local deployments this may need to be set manually
    pub fn auth_url(mut self, auth_url: impl Into<String>) -> Self {
        self.auth_url = Some(auth_url.into());
        self
    }

    /// Sets the max messages we can send in a second.
    /// Default: 10
    pub fn max_events_per_second(mut self, count: usize) -> Self {
        self.max_events_per_second = count;
        self
    }

    pub fn encode(mut self, encode: impl Fn(RealtimeMessage) -> RealtimeMessage + 'static) -> Self {
        self.encode = Some(Box::new(encode));
        self
    }

    pub fn decode(mut self, decode: impl Fn(RealtimeMessage) -> RealtimeMessage + 'static) -> Self {
        self.decode = Some(Box::new(decode));
        self
    }

    /// Consume the [Self] and return a configured [RealtimeClient]
    pub fn build(self) -> RealtimeClient {
        RealtimeClient {
            headers: self.headers,
            params: self.params,
            heartbeat_interval: self.heartbeat_interval,
            encode: self.encode,
            decode: self.decode,
            reconnect_interval: self.reconnect_interval,
            reconnect_max_attempts: self.reconnect_max_attempts,
            connection_timeout: self.connection_timeout,
            endpoint: self.endpoint,
            access_token: self.access_token,
            max_events_per_second: self.max_events_per_second,
            next_ref: Uuid::new_v4(),
            ..Default::default()
        }
    }
}

fn backoff(attempts: usize) -> Duration {
    let times: Vec<u64> = vec![0, 1, 2, 5, 10];

    Duration::from_secs(times[attempts.min(times.len() - 1)])
}
