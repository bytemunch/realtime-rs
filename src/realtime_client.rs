use std::fmt::Debug;
use std::sync::Arc;
use std::time::SystemTime;
use std::{collections::HashMap, time::Duration};

use tokio::runtime::Runtime;
use tokio::sync::mpsc::error::SendError;
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
use crate::realtime_channel::{ChannelManager, ChannelManagerMessage};
use crate::Responder;

/// Connection state of [RealtimeClient]
#[derive(PartialEq, Debug, Default, Clone, Copy)]
pub enum ClientState {
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

pub enum ClientManagerMessage {
    Connect {
        res: Responder<()>,
    },
    GetWsTx {
        res: Responder<UnboundedSender<RealtimeMessage>>,
    },
    GetAccessToken {
        res: Responder<String>,
    },
    GetState {
        res: Responder<ClientState>,
    },
    AddChannel {
        manager: ChannelManager,
        res: Responder<ChannelManager>,
    },
}

#[derive(Clone, Debug)]
pub struct ClientManager {
    tx: UnboundedSender<ClientManagerMessage>,
    rt: Arc<Runtime>,
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
    pub async fn get_state(&self) -> Result<ClientState, oneshot::error::RecvError> {
        let (tx, rx) = oneshot::channel();
        let _ = self.send(ClientManagerMessage::GetState { res: tx });
        rx.await
    }
    pub fn get_rt(&self) -> Arc<Runtime> {
        self.rt.clone()
    }
    pub(crate) async fn get_ws_tx(
        &self,
    ) -> Result<UnboundedSender<RealtimeMessage>, oneshot::error::RecvError> {
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
    pub fn to_sync(self) -> ClientManagerSync {
        ClientManagerSync { inner: self }
    }
}

#[derive(Clone)]
pub struct ClientManagerSync {
    inner: ClientManager,
}

impl ClientManagerSync {
    pub fn get_rt(&self) -> Arc<Runtime> {
        self.inner.rt.clone()
    }
    pub fn connect(&self) {
        self.inner.rt.block_on(self.inner.connect());
    }
    pub fn get_ws_tx(&self) -> Result<UnboundedSender<RealtimeMessage>, oneshot::error::RecvError> {
        self.inner.rt.block_on(self.inner.get_ws_tx())
    }
    pub fn get_state(&self) -> Result<ClientState, oneshot::error::RecvError> {
        self.inner.rt.block_on(self.inner.get_state())
    }
    pub fn get_access_token(&self) -> Result<String, oneshot::error::RecvError> {
        self.inner.rt.block_on(self.inner.get_access_token())
    }
    pub fn add_channel(
        &self,
        channel_manager: ChannelManager,
    ) -> Result<ChannelManager, oneshot::error::RecvError> {
        self.inner
            .rt
            .block_on(self.inner.add_channel(channel_manager))
    }
    pub fn into_inner(self) -> ClientManager {
        self.inner
    }
}

/// Synchronous websocket client that interfaces with Supabase Realtime
struct RealtimeClient {
    pub(crate) access_token: String, // TODO wrap with Arc
    state: Arc<Mutex<ClientState>>,
    ws_tx: Option<mpsc::UnboundedSender<RealtimeMessage>>,
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
    encode: Option<Box<fn(RealtimeMessage) -> RealtimeMessage>>,
    decode: Option<Box<fn(RealtimeMessage) -> RealtimeMessage>>,
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
    rt: Arc<Runtime>,
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
                ClientManagerMessage::GetState { res } => {
                    let s = self.state.lock().await;
                    res.send(s.clone()).unwrap();
                }
            }
        }
    }

    /// Attempt to create a websocket connection with the server
    async fn connect(&mut self) -> Result<&mut RealtimeClient, ConnectError> {
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
        // Clear current async tasks
        for t in &mut self.join_handles {
            t.abort();
        }

        self.join_handles.clear();

        let request = self.build_request()?;

        let mut reconnect_attempts = 0;
        loop {
            let (ws_tx_tx, ws_tx_rx) = mpsc::unbounded_channel();

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

            let encode = self.encode.clone();

            let send_task = self.rt.spawn(async move {
                let sender = UnboundedReceiverStream::new(ws_tx_rx)
                    .map(|mut x: RealtimeMessage| {
                        // TODO encode here
                        if let Some(encode) = encode.clone() {
                            x = encode(x);
                        }
                        println!("[SEND] {:?}", x);
                        Ok(x.into())
                    })
                    .forward(write);
                pin_mut!(sender);
                let _ = sender.await;
            });

            let channel_list = self.channels.clone();
            let recv_state = self.state.clone();
            let manager = self.manager.clone();
            let decode = self.decode.clone();

            let recieve_task = self.rt.spawn(async move {
                loop {
                    while let Some(msg) = read.next().await {
                        if let Err(_err) = msg {
                            println!("Disconnected!");
                            let mut state = recv_state.lock().await;
                            *state = ClientState::Reconnect;
                            continue;
                        }

                        let Ok(mut msg) = serde_json::from_str::<RealtimeMessage>(
                            msg.as_ref().unwrap().to_text().unwrap(), // TODO error handling here,
                                                                      // disconnect detect
                        ) else {
                            continue;
                        };

                        println!("[RECV] {:?}", msg);

                        if let Some(decode) = decode.clone() {
                            msg = decode(msg);
                        }

                        let list = channel_list.lock().await;

                        for channel in &*list {
                            if *channel.get_topic().await != msg.topic {
                                continue;
                            }
                            let _ = channel.get_tx().await.send(msg.clone());
                        }
                    }

                    let mut state = recv_state.lock().await;
                    if *state == ClientState::Reconnect {
                        println!("Reconnecting...");
                        *state = ClientState::Reconnecting;
                        drop(state);
                        manager.connect().await;
                    }
                }
            });

            let hb_tx = ws_tx_tx.clone();
            let hb_ivl = self.heartbeat_interval.clone(); // TODO heartbeat interval can be moved here,
                                                          // no need to keep on RealtimeClient
            let heartbeat_task = self.rt.spawn(async move {
                loop {
                    sleep(hb_ivl).await;
                    let _ = hb_tx.send(RealtimeMessage::heartbeat());
                }
            });

            {
                let mut state = self.state.lock().await;
                *state = ClientState::Open;
            }

            self.join_handles.push(send_task);
            self.join_handles.push(recieve_task);
            self.join_handles.push(heartbeat_task);

            self.ws_tx = Some(ws_tx_tx);

            let mut channels = self.channels.lock().await;

            // TODO filter out finished channels here

            for manager in channels.iter_mut() {
                let (cc_tx, cc_rx) = oneshot::channel();
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

    async fn add_channel(&mut self, channel: ChannelManager) -> ChannelManager {
        let list = self.channels.clone();

        let mut list = list.lock().await;

        list.push(channel.clone());

        channel
    }

    /// Use provided JWT to authorize future requests from this client and all channels
    fn set_auth(&mut self, access_token: String) {
        self.access_token = access_token.clone();

        // TODO channel should reference client auth directly
    }

    // TODO look into if middleware is needed?
    /// Add a callback to run mutably on recieved [RealtimeMessage]s before any other registered
    /// callbacks.
    fn add_middleware(
        &mut self,
        middleware: Box<dyn Fn(RealtimeMessage) -> RealtimeMessage + Send>,
    ) -> Uuid {
        // TODO user defined middleware ordering
        let uuid = Uuid::new_v4();
        self.middleware.insert(uuid, middleware);
        uuid
    }

    /// Remove middleware by it's [Uuid]
    fn remove_middleware(&mut self, uuid: Uuid) -> &mut RealtimeClient {
        self.middleware.remove(&uuid);
        self
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
    encode: Option<Box<fn(RealtimeMessage) -> RealtimeMessage>>, // TODO interceptor type
    decode: Option<Box<fn(RealtimeMessage) -> RealtimeMessage>>,
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

    pub fn encode(mut self, encode: fn(RealtimeMessage) -> RealtimeMessage) -> Self {
        self.encode = Some(Box::new(encode));
        self
    }

    pub fn decode(mut self, decode: fn(RealtimeMessage) -> RealtimeMessage) -> Self {
        self.decode = Some(Box::new(decode));
        self
    }

    /// Consume the [Self] and return a configured [RealtimeClient]
    pub fn build(self) -> ClientManager {
        let (mgr_tx, mgr_rx) = mpsc::unbounded_channel::<ClientManagerMessage>();
        let tx = mgr_tx.clone();

        // Needs to be multithread to work with .spawn()
        // Breaks WASM support tho ;_;
        let rt = Arc::new(
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .unwrap(),
        );

        let manager = ClientManager { tx, rt: rt.clone() };

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
            state: Arc::new(Mutex::new(ClientState::Closed)),
            ws_tx: None,
            channels: Arc::new(Mutex::new(Vec::new())),
            messages_this_second: Vec::new(),
            middleware: HashMap::new(),
            join_handles: Vec::new(),
            manager_channel: (mgr_tx, mgr_rx),
            manager: manager.clone(),
            rt: rt.clone(),
        };

        let _handle = rt.spawn(async move { client.manager_recv().await });

        manager
    }
}

fn backoff(attempts: usize) -> Duration {
    let times: Vec<u64> = vec![0, 1, 2, 5, 10];

    Duration::from_secs(times[attempts.min(times.len() - 1)])
}
