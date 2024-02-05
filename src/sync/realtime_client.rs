use std::fmt::{Debug, Display};
use std::sync::Arc;
use std::time::SystemTime;
use std::{collections::HashMap, time::Duration};

use tokio::sync::mpsc::error::{SendError, TryRecvError};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio::task::JoinHandle;
use tokio::time::sleep;
use tokio_stream::wrappers::UnboundedReceiverStream;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::{HeaderMap, HeaderValue, Request, Uri};
use tokio_tungstenite::tungstenite::Message;

use uuid::Uuid;

use futures_util::{pin_mut, StreamExt};

use crate::message::RealtimeMessage;
use crate::sync::ChannelManagerMessage;

use super::{ChannelManager, Responder};

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
pub(crate) struct MessageChannel(
    (
        UnboundedSender<RealtimeMessage>,
        UnboundedReceiver<RealtimeMessage>,
    ),
);

impl Default for MessageChannel {
    fn default() -> Self {
        Self(mpsc::unbounded_channel())
    }
}

pub enum ClientManagerMessage {
    Connect {
        res: Responder<()>,
    },
    GetWsTx {
        res: Responder<UnboundedSender<Message>>,
    },
    GetAccessToken {
        res: Responder<String>,
    },
    AddChannel {
        manager: ChannelManager,
        res: Responder<ChannelManager>,
    },
}

#[derive(Clone, Debug)]
pub struct ClientManager {
    tx: UnboundedSender<ClientManagerMessage>,
}

impl ClientManager {
    fn send(&self, message: ClientManagerMessage) -> Result<(), SendError<ClientManagerMessage>> {
        self.tx.send(message)
    }

    pub async fn connect(&self) {
        let (tx, rx) = oneshot::channel();
        let _ = self.send(ClientManagerMessage::Connect { res: tx });
        rx.await.unwrap()
    }

    pub async fn get_ws_tx(&self) -> Result<UnboundedSender<Message>, oneshot::error::RecvError> {
        let (tx, rx) = oneshot::channel();
        let _ = self.send(ClientManagerMessage::GetWsTx { res: tx });
        rx.await
    }

    pub async fn get_access_token(&self) -> Result<String, oneshot::error::RecvError> {
        let (tx, rx) = oneshot::channel();
        let _ = self.send(ClientManagerMessage::GetAccessToken { res: tx });
        rx.await
    }

    pub async fn add_channel(
        &self,
        channel_manager: ChannelManager,
    ) -> Result<ChannelManager, oneshot::error::RecvError> {
        let (tx, rx) = oneshot::channel();
        let _ = self.send(ClientManagerMessage::AddChannel {
            res: tx,
            manager: channel_manager,
        });
        rx.await
    }
}

/// Synchronous websocket client that interfaces with Supabase Realtime
pub struct RealtimeClient {
    pub(crate) access_token: String, // TODO wrap with Arc
    state: Arc<Mutex<ConnectionState>>,
    ws_tx: Option<mpsc::UnboundedSender<Message>>,
    ws_rx: Option<mpsc::UnboundedReceiver<Message>>,
    channels: Arc<Mutex<Vec<ChannelManager>>>,
    messages_this_second: Vec<SystemTime>,
    next_ref: Uuid,
    middleware: HashMap<Uuid, Box<dyn Fn(RealtimeMessage) -> RealtimeMessage + Send>>,
    // threads
    join_handles: Vec<JoinHandle<()>>,
    // builder options
    headers: HeaderMap,
    params: Option<HashMap<String, String>>,
    heartbeat_interval: Duration,
    encode: Option<Box<dyn Fn(RealtimeMessage) -> RealtimeMessage + Send>>,
    decode: Option<Box<dyn Fn(RealtimeMessage) -> RealtimeMessage + Send>>,
    reconnect_interval: ReconnectFn,
    reconnect_max_attempts: usize,
    connection_timeout: Duration,
    endpoint: String,
    max_events_per_second: usize,
    manager_channel: (
        UnboundedSender<ClientManagerMessage>,
        UnboundedReceiver<ClientManagerMessage>,
    ),
    manager: ClientManager,
}

impl Debug for RealtimeClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // TODO this is horrid
        f.write_fmt(format_args!("todo"))
    }
}

impl RealtimeClient {
    async fn manager_recv(&mut self) {
        while let Some(control_message) = self.manager_channel.1.recv().await {
            match control_message {
                ClientManagerMessage::Connect { res } => {
                    let _ = self.connect().await;
                    let _ = res.send(());
                }
                ClientManagerMessage::GetWsTx { res } => {
                    let _ = res.send(self.ws_tx.clone().unwrap());
                }
                ClientManagerMessage::GetAccessToken { res } => {
                    let _ = res.send(self.access_token.clone());
                }
                ClientManagerMessage::AddChannel { manager, res } => {
                    let added = self.add_channel(manager).await;

                    let _ = res.send(added);
                }
            }
        }
    }
    /// Returns a new [RealtimeClientBuilder] with provided `endpoint` and `access_token`
    pub fn builder(
        endpoint: impl Into<String>,
        access_token: impl Into<String>,
    ) -> RealtimeClientBuilder {
        RealtimeClientBuilder::new(endpoint, access_token)
    }

    /// Returns this client's [ConnectionState]
    pub async fn get_status(&self) -> ConnectionState {
        let state = self.state.lock().await;
        *state
    }

    /// Attempt to create a websocket connection with the server
    /// TODO CODE
    /// ```
    pub async fn connect(&mut self) -> Result<&mut RealtimeClient, ConnectError> {
        let _ = self.connect_ws().await;

        Ok(self)
    }

    fn build_request(&self) -> Result<Request<()>, ConnectError> {
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

        Ok(request)
    }

    async fn connect_ws(&mut self) -> Result<(), ConnectError> {
        println!("Connecting...");
        // Clear current threads
        for t in &mut self.join_handles {
            t.abort();
        }

        self.join_handles.clear();

        let request = self.build_request()?;

        let mut reconnect_attempts = 0;
        loop {
            // change all channel client_tx to new ones?

            let (ws_tx_tx, ws_tx_rx) = mpsc::unbounded_channel();
            let (ws_rx_tx, ws_rx_rx) = mpsc::unbounded_channel();

            let Ok((ws_stream, _res)) = connect_async(request.clone()).await else {
                reconnect_attempts += 1;
                println!(
                    "Connection failed. Retrying (attempt #{})",
                    reconnect_attempts
                );
                sleep(self.reconnect_interval.0(reconnect_attempts)).await;
                continue;
            };

            println!("WebSocket handshake has been successfully completed");

            let (write, mut read) = ws_stream.split();

            let send_thread = tokio::spawn(async move {
                let sender = UnboundedReceiverStream::new(ws_tx_rx)
                    .map(|x| {
                        // TODO encode here
                        println!("[SEND] {:?}", x);
                        Ok(x)
                    })
                    .forward(write);
                pin_mut!(sender);
                let _ = sender.await;
            });

            let channel_list = self.channels.clone();
            let recv_state = self.state.clone();
            let manager = self.manager.clone();

            let recieve_thread = tokio::spawn(async move {
                loop {
                    while let Some(msg) = read.next().await {
                        if let Err(_err) = msg {
                            println!("Disconnected!");
                            let mut state = recv_state.lock().await;
                            *state = ConnectionState::Reconnect;
                            continue;
                        }
                        // TODO decode here
                        let Ok(rt_msg) = serde_json::from_str::<RealtimeMessage>(
                            msg.as_ref().unwrap().to_text().unwrap(), // TODO error handling here,
                                                                      // disconnect detect
                        ) else {
                            continue;
                        };

                        let Ok(msg) = msg else {
                            continue;
                        };

                        let list = channel_list.lock().await;

                        for channel in &*list {
                            if *channel.get_topic().await != rt_msg.topic {
                                continue;
                            }
                            let _ = channel.get_tx().await.send(msg.clone());
                        }

                        let _ = ws_rx_tx.send(msg);
                    }

                    let mut state = recv_state.lock().await;
                    if *state == ConnectionState::Reconnect {
                        println!("Reconnecting...");
                        *state = ConnectionState::Reconnecting;
                        drop(state);
                        manager.connect().await;
                    }
                }
            });

            let hb_tx = ws_tx_tx.clone();
            let hb_ivl = self.heartbeat_interval.clone(); // TODO heartbeat interval can be moved here,
                                                          // no need to keep on RealtimeClient
            let heartbeat_thread = tokio::spawn(async move {
                loop {
                    sleep(hb_ivl).await;
                    let _ = hb_tx.send(RealtimeMessage::heartbeat().into());
                }
            });

            {
                let mut state = self.state.lock().await;
                *state = ConnectionState::Open;
            }

            self.join_handles.push(send_thread);
            self.join_handles.push(recieve_thread);
            self.join_handles.push(heartbeat_thread);

            self.ws_tx = Some(ws_tx_tx);
            self.ws_rx = Some(ws_rx_rx);

            let mut channels = self.channels.lock().await;

            // TODO filter out finished channels here

            for manager in channels.iter_mut() {
                let (cc_tx, cc_rx) = ChannelManager::oneshot();
                let _ = manager.send(ChannelManagerMessage::ClientTx {
                    new_tx: self.ws_tx.clone().unwrap(),
                    res: cc_tx,
                });

                let _ = cc_rx.await;

                let _ = manager.subscribe();
            }

            break;
        }

        Ok(())
    }

    async fn ws_recv(&mut self, manager: ClientManager) {
        let state = self.state.clone();

        loop {
            while let Some(msg) = self.ws_rx.as_mut().unwrap().recv().await {
                println!("[RECV] {:?}", msg);
            }

            println!("Recv fail");

            let mut state = state.lock().await;
            if *state == ConnectionState::Reconnect {
                println!("Reconnecting...");
                *state = ConnectionState::Reconnecting;
                drop(state);
                let _ = manager.connect();
            }
        }
    }

    pub async fn send(&mut self, msg: RealtimeMessage) -> Result<(), SendError<Message>> {
        self.ws_tx.as_mut().unwrap().send(msg.into())
    }

    /// Use provided JWT to authorize future requests from this client and all channels
    pub fn set_auth(&mut self, access_token: String) {
        self.access_token = access_token.clone();

        // TODO channel should reference client auth directly
    }

    // TODO look into if middleware is needed?
    /// Add a callback to run mutably on recieved [RealtimeMessage]s before any other registered
    /// callbacks.
    pub fn add_middleware(
        &mut self,
        middleware: Box<dyn Fn(RealtimeMessage) -> RealtimeMessage + Send>,
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

    async fn add_channel(&mut self, channel: ChannelManager) -> ChannelManager {
        let list = self.channels.clone();

        let mut list = list.lock().await;

        list.push(channel.clone());

        channel
    }

    fn run_middleware(&self, mut message: RealtimeMessage) -> RealtimeMessage {
        for middleware in self.middleware.values() {
            message = middleware(message)
        }
        message
    }
}

/// Takes a `Box<dyn Fn(usize) -> Duration>`
/// The provided function should take a count of reconnect attempts and return a [Duration] to wait
/// until the next attempt is made.
pub struct ReconnectFn(pub Box<dyn Fn(usize) -> Duration + Send>);

impl ReconnectFn {
    pub fn new(f: impl Fn(usize) -> Duration + 'static + Send) -> Self {
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
    encode: Option<Box<dyn Fn(RealtimeMessage) -> RealtimeMessage + Send>>,
    decode: Option<Box<dyn Fn(RealtimeMessage) -> RealtimeMessage + Send>>,
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

    pub fn encode(
        mut self,
        encode: impl Fn(RealtimeMessage) -> RealtimeMessage + 'static + Send,
    ) -> Self {
        self.encode = Some(Box::new(encode));
        self
    }

    pub fn decode(
        mut self,
        decode: impl Fn(RealtimeMessage) -> RealtimeMessage + 'static + Send,
    ) -> Self {
        self.decode = Some(Box::new(decode));
        self
    }

    /// Consume the [Self] and return a configured [RealtimeClient]
    pub async fn build(self) -> ClientManager {
        let (mgr_tx, mgr_rx) = mpsc::unbounded_channel::<ClientManagerMessage>();
        let tx = mgr_tx.clone();

        let manager = ClientManager { tx };

        let manager2 = manager.clone();

        let mut client = RealtimeClient {
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
            state: Arc::new(Mutex::new(ConnectionState::Closed)),
            ws_tx: None,
            ws_rx: None,
            channels: Arc::new(Mutex::new(Vec::new())),
            messages_this_second: Vec::new(),
            middleware: HashMap::new(),
            join_handles: Vec::new(),
            manager_channel: (mgr_tx, mgr_rx),
            manager,
        };

        // client.ws_recv(manager.clone()).await;
        let handle = tokio::spawn(async move { client.manager_recv().await });

        manager2
    }
}

fn backoff(attempts: usize) -> Duration {
    let times: Vec<u64> = vec![0, 1, 2, 5, 10];

    Duration::from_secs(times[attempts.min(times.len() - 1)])
}
