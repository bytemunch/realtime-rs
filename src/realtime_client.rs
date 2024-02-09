use std::fmt::Debug;
use std::sync::Arc;
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

use futures_util::{pin_mut, StreamExt};

use crate::message::RealtimeMessage;
use crate::realtime_channel::{ChannelManager, ChannelManagerMessage, ChannelState};
use crate::Responder;

pub type Interceptor = fn(RealtimeMessage) -> RealtimeMessage;

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

pub(crate) enum ClientManagerMessage {
    Connect {
        res: Responder<()>,
    },
    Disconnect {
        res: Responder<RealtimeClientBuilder>,
    },
    GetWsTx {
        res: Responder<UnboundedSender<RealtimeMessage>>,
    },
    GetAccessToken {
        res: Responder<String>,
    },
    GetAccessTokenArc {
        res: Responder<Arc<Mutex<String>>>,
    },
    GetState {
        res: Responder<ClientState>,
    },
    SetAccessToken {
        res: Responder<String>,
        access_token: String,
    },
    AddChannel {
        manager: ChannelManager,
        res: Responder<ChannelManager>,
    },
}

/// Manager struct for a [RealtimeClient]
///
/// Returned by [RealtimeClientBuilder::connect()]
// TODO code example showing creation
#[derive(Clone, Debug)]
pub struct ClientManager {
    tx: UnboundedSender<ClientManagerMessage>,
    rt: Arc<Runtime>,
}

impl ClientManager {
    fn send(&self, message: ClientManagerMessage) -> Result<(), SendError<ClientManagerMessage>> {
        self.tx.send(message)
    }
    /// Connect to the websocket server
    // TODO example code
    pub async fn connect(&self) {
        let (tx, rx) = oneshot::channel();
        let _ = self.send(ClientManagerMessage::Connect { res: tx });
        rx.await.unwrap()
    }
    /// Disconnect the client
    /// Returns a preconfigured [RealtimeClientBuilder] for modification or reinstantiation
    pub async fn disconnect(&self) -> Result<RealtimeClientBuilder, oneshot::error::RecvError> {
        let (tx, rx) = oneshot::channel();
        let _ = self.send(ClientManagerMessage::Disconnect { res: tx });
        rx.await
    }
    /// Returns the current [ClientState]
    pub async fn get_state(&self) -> Result<ClientState, oneshot::error::RecvError> {
        let (tx, rx) = oneshot::channel();
        let _ = self.send(ClientManagerMessage::GetState { res: tx });
        rx.await
    }
    /// Returns an Arc referencing the client's internal tokio runtime
    pub fn get_rt(&self) -> Arc<Runtime> {
        self.rt.clone()
    }
    /// Returns the current access token used by this client
    pub async fn get_access_token(&self) -> Result<String, oneshot::error::RecvError> {
        let (tx, rx) = oneshot::channel();
        let _ = self.send(ClientManagerMessage::GetAccessToken { res: tx });
        rx.await
    }
    /// Returns an Arc<Mutex<String>> referencing the client's access token
    pub async fn get_access_token_arc(
        &self,
    ) -> Result<Arc<Mutex<String>>, oneshot::error::RecvError> {
        let (tx, rx) = oneshot::channel();
        let _ = self.send(ClientManagerMessage::GetAccessTokenArc { res: tx });
        rx.await
    }
    /// Modify the client's access token
    /// This change cascades through all connected channels and sends the appropriate messages to
    /// the server
    pub async fn set_access_token(
        &self,
        access_token: String,
    ) -> Result<String, oneshot::error::RecvError> {
        let (tx, rx) = oneshot::channel();
        let _ = self.send(ClientManagerMessage::SetAccessToken {
            res: tx,
            access_token,
        });
        rx.await
    }
    /// Return a sync wrapper [ClientManagerSync] for this manager
    pub fn to_sync(self) -> ClientManagerSync {
        ClientManagerSync { inner: self }
    }
    pub(crate) async fn add_channel(
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
    pub(crate) async fn get_ws_tx(
        &self,
    ) -> Result<UnboundedSender<RealtimeMessage>, oneshot::error::RecvError> {
        let (tx, rx) = oneshot::channel();
        let _ = self.send(ClientManagerMessage::GetWsTx { res: tx });
        rx.await
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
    /// Connect to the websocket server
    // TODO example code
    pub fn connect(&self) {
        self.inner.rt.block_on(self.inner.connect());
    }
    /// Disconnect the client
    /// Returns a preconfigured [RealtimeClientBuilder] for modification or reinstantiation
    pub fn disconnect(&self) -> Result<RealtimeClientBuilder, oneshot::error::RecvError> {
        self.inner.rt.block_on(self.inner.disconnect())
    }
    pub fn get_ws_tx(&self) -> Result<UnboundedSender<RealtimeMessage>, oneshot::error::RecvError> {
        self.inner.rt.block_on(self.inner.get_ws_tx())
    }
    /// Returns the current [ClientState]
    pub fn get_state(&self) -> Result<ClientState, oneshot::error::RecvError> {
        self.inner.rt.block_on(self.inner.get_state())
    }
    /// Returns the current access token used by this client
    pub fn get_access_token(&self) -> Result<String, oneshot::error::RecvError> {
        self.inner.rt.block_on(self.inner.get_access_token())
    }
    /// Returns an Arc referencing the client's internal tokio runtime
    pub fn get_access_token_arc(&self) -> Result<Arc<Mutex<String>>, oneshot::error::RecvError> {
        self.inner.rt.block_on(self.inner.get_access_token_arc())
    }
    /// Modify the client's access token
    /// This change cascades through all connected channels and sends the appropriate messages to
    /// the server
    pub fn set_access_token(
        &self,
        access_token: String,
    ) -> Result<String, oneshot::error::RecvError> {
        self.inner
            .rt
            .block_on(self.inner.set_access_token(access_token))
    }
    /// Unwrap the inner [ClientManager]. Consumes self.
    pub fn to_async(self) -> ClientManager {
        self.inner
    }
    pub(crate) fn add_channel(
        &self,
        channel_manager: ChannelManager,
    ) -> Result<ChannelManager, oneshot::error::RecvError> {
        self.inner
            .rt
            .block_on(self.inner.add_channel(channel_manager))
    }
}

/// Synchronous websocket client that interfaces with Supabase Realtime
struct RealtimeClient {
    pub(crate) access_token: Arc<Mutex<String>>,
    state: Arc<Mutex<ClientState>>,
    ws_tx: Option<mpsc::UnboundedSender<RealtimeMessage>>,
    channels: Arc<Mutex<Vec<ChannelManager>>>,
    // threads
    join_handles: Vec<JoinHandle<()>>,
    // builder options
    headers: HeaderMap,
    params: Option<HashMap<String, String>>,
    heartbeat_interval: Duration,
    encode: Option<Box<Interceptor>>,
    decode: Option<Box<Interceptor>>,
    reconnect_interval: ReconnectFn,
    reconnect_max_attempts: usize,
    endpoint: String,
    manager_channel: (
        UnboundedSender<ClientManagerMessage>,
        UnboundedReceiver<ClientManagerMessage>,
    ),
    manager: ClientManager,
    rt: Arc<Runtime>,
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
                    let token = self.access_token.lock().await;
                    let _ = res.send(token.clone());
                }
                ClientManagerMessage::SetAccessToken { access_token, res } => {
                    let mut token = self.access_token.lock().await;
                    *token = access_token;

                    let channels = self.channels.lock().await;
                    for c in channels.iter() {
                        c.reauth().await.unwrap();
                    }
                    let _ = res.send(token.clone());
                }
                ClientManagerMessage::AddChannel { manager, res } => {
                    let added = self.add_channel(manager).await;

                    let _ = res.send(added);
                }
                ClientManagerMessage::GetState { res } => {
                    let s = self.state.lock().await;
                    res.send(*s).unwrap();
                }
                ClientManagerMessage::GetAccessTokenArc { res } => {
                    res.send(self.access_token.clone()).unwrap();
                }
                ClientManagerMessage::Disconnect { res } => {
                    res.send(self.disconnect().await).unwrap();
                }
            }
        }
    }

    /// Attempt to create a websocket connection with the server
    async fn connect(&mut self) -> Result<&mut RealtimeClient, ConnectError> {
        let _ = self.connect_ws().await;

        Ok(self)
    }

    async fn build_request(&self) -> Result<Request<()>, ConnectError> {
        let token = self.access_token.lock().await;
        let uri: Uri = match format!(
            "{}/realtime/v1/websocket?apikey={}&vsn=1.0.0",
            self.endpoint, *token
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

        let auth: HeaderValue = format!("Bearer {}", *token)
            .parse()
            .expect("malformed access token?");
        headers.insert("Authorization", auth);

        let xci: HeaderValue = "realtime-rs/0.1.0".to_string().parse().unwrap();
        headers.insert("X-Client-Info", xci);

        headers.extend(self.headers.clone());

        Ok(request)
    }

    fn clear_tasks(&mut self) {
        // Clear current async tasks
        for t in &mut self.join_handles {
            t.abort();
        }

        self.join_handles.clear();
    }

    async fn clear_channels(&mut self) {
        let c = self.channels.lock().await;
        for c in c.iter() {
            c.unsubscribe().await.unwrap().unwrap();
        }
    }

    async fn disconnect(&mut self) -> RealtimeClientBuilder {
        self.clear_channels().await;
        self.clear_tasks();

        let mut state = self.state.lock().await;
        *state = ClientState::Closed;
        println!("Disconnected!");

        let access_token = self.access_token.lock().await;

        RealtimeClientBuilder {
            heartbeat_interval: self.heartbeat_interval,
            params: self.params.clone(),
            encode: self.encode.clone(),
            decode: self.decode.clone(),
            headers: self.headers.clone(),
            endpoint: self.endpoint.clone(),
            access_token: access_token.clone(),
            reconnect_interval: self.reconnect_interval.clone(),
            reconnect_max_attempts: self.reconnect_max_attempts,
        }
    }

    async fn connect_ws(&mut self) -> Result<(), ConnectError> {
        println!("Connecting...");

        self.clear_tasks();

        let request = self.build_request().await?;

        let mut reconnect_attempts = 0;
        loop {
            let (ws_tx_tx, ws_tx_rx) = mpsc::unbounded_channel();

            let Ok((ws_stream, _res)) = connect_async(request.clone()).await else {
                reconnect_attempts += 1;
                if reconnect_attempts >= self.reconnect_max_attempts {
                    println!(
                        "Connection failed, max retries exceeded ({}/{})",
                        reconnect_attempts, self.reconnect_max_attempts
                    );
                    return Err(ConnectError::MaxRetries);
                }
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
                        if let Some(encode) = encode.clone() {
                            x = encode(x);
                        }
                        // TODO throttling. READING: drop or queue throttled messages? check what
                        // official clients do.
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
                            msg.as_ref().unwrap().to_text().unwrap(),
                        ) else {
                            continue;
                        };

                        println!("[RECV] {:?}", msg);

                        if let Some(decode) = decode.clone() {
                            msg = decode(msg);
                        }

                        let mut list = channel_list.lock().await;

                        *list = tokio_stream::iter(list.clone())
                            .filter_map(|c| async {
                                if c.get_state().await.unwrap() != ChannelState::Closed {
                                    Some(c)
                                } else {
                                    None
                                }
                            })
                            .collect()
                            .await;

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
            let hb_ivl = self.heartbeat_interval;
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

            for manager in channels.iter_mut() {
                let (cc_tx, cc_rx) = oneshot::channel();
                let _ = manager.send(ChannelManagerMessage::ClientTx {
                    new_tx: self.ws_tx.clone().unwrap(),
                    res: cc_tx,
                });

                let _ = cc_rx.await;

                manager.subscribe();
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
}

/// Takes a `Box<dyn Fn(usize) -> Duration>`
/// The provided function should take a count of reconnect attempts and return a [Duration] to wait
/// until the next attempt is made.
#[derive(Clone, Debug)]
pub struct ReconnectFn(pub Box<fn(usize) -> Duration>);

impl ReconnectFn {
    pub fn new(f: fn(usize) -> Duration) -> Self {
        Self(Box::new(f))
    }
}

impl Default for ReconnectFn {
    fn default() -> Self {
        Self(Box::new(backoff))
    }
}

/// Builder struct for [RealtimeClient]
#[derive(Debug)] // for .unwrap in manager? Hmmge
pub struct RealtimeClientBuilder {
    headers: HeaderMap,
    params: Option<HashMap<String, String>>,
    heartbeat_interval: Duration,
    encode: Option<Box<Interceptor>>,
    decode: Option<Box<Interceptor>>,
    reconnect_interval: ReconnectFn,
    reconnect_max_attempts: usize,
    endpoint: String,
    access_token: String,
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
            endpoint: endpoint.into(),
            access_token: anon_key.into(),
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
    /// # use std::time::Duration;
    ///   fn backoff(attempts: usize) -> Duration {
    ///       let times: Vec<u64> = vec![0, 1, 2, 5, 10];
    ///       Duration::from_secs(times[attempts.min(times.len() - 1)])
    ///   }
    pub fn reconnect_interval(mut self, reconnect_interval: ReconnectFn) -> Self {
        self.reconnect_interval = reconnect_interval;
        self
    }

    /// Configure the number of recconect attempts to be made before erroring
    pub fn reconnect_max_attempts(mut self, max_attempts: usize) -> Self {
        self.reconnect_max_attempts = max_attempts;
        self
    }

    pub fn encode(mut self, encode: Interceptor) -> Self {
        self.encode = Some(Box::new(encode));
        self
    }

    pub fn decode(mut self, decode: Interceptor) -> Self {
        self.decode = Some(Box::new(decode));
        self
    }

    /// Consume the [Self] and return a configured [ClientManager]
    pub fn connect(self) -> ClientManager {
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
            endpoint: self.endpoint,
            access_token: Arc::new(Mutex::new(self.access_token)),
            state: Arc::new(Mutex::new(ClientState::Closed)),
            ws_tx: None,
            channels: Arc::new(Mutex::new(Vec::new())),
            join_handles: Vec::new(),
            manager_channel: (mgr_tx, mgr_rx),
            manager: manager.clone(),
            rt: rt.clone(),
        };

        let _handle = rt.spawn(async move {
            client.connect().await.unwrap();
            client.manager_recv().await;
        });

        manager
    }
}

fn backoff(attempts: usize) -> Duration {
    let times: Vec<u64> = vec![0, 1, 2, 5, 10];

    Duration::from_secs(times[attempts.min(times.len() - 1)])
}
