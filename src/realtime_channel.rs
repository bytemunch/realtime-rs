use crate::realtime_client::ClientManager;
use crate::realtime_client::ClientManagerSync;
use crate::realtime_presence::PresenceCallbackMap;
use crate::realtime_presence::RealtimePresence;
use crate::Responder;

use serde_json::Value;
use tokio::{
    runtime::Runtime,
    sync::{
        mpsc::{self, error::SendError, UnboundedReceiver, UnboundedSender},
        oneshot::{self, error::RecvError},
        Mutex,
    },
    task::JoinHandle,
};
use uuid::Uuid;

use crate::message::{
    payload::{
        AccessTokenPayload, BroadcastConfig, BroadcastPayload, JoinConfig, JoinPayload, Payload,
        PayloadStatus, PostgresChange, PostgresChangesEvent, PostgresChangesPayload,
        PresenceConfig,
    },
    presence::{PresenceEvent, PresenceState},
    MessageEvent, PostgresChangeFilter, RealtimeMessage,
};

use std::fmt::Debug;
use std::{collections::HashMap, sync::Arc};

type CdcCallback = (
    PostgresChangeFilter,
    Box<dyn FnMut(&PostgresChangesPayload) + Send>,
);
type BroadcastCallback = Box<dyn FnMut(&HashMap<String, Value>) + Send>;
pub(crate) type PresenceCallback = Box<dyn Fn(String, PresenceState, PresenceState) + Send>;

/// Channel states
#[derive(PartialEq, Clone, Copy, Debug)]
pub enum ChannelState {
    Closed,
    Errored,
    Joined,
    Joining,
    Leaving,
}

/// Error for channel send failures
#[derive(Debug)]
pub enum ChannelSendError {
    NoChannel,
    SendError(SendError<RealtimeMessage>),
    ChannelError(ChannelState),
}

pub enum ChannelManagerMessage {
    Subscribe,
    Unsubscribe {
        res: Responder<Result<ChannelState, ChannelSendError>>,
    },
    SubscribeBlocking {
        res: Responder<()>,
    },
    Broadcast {
        payload: BroadcastPayload,
    },
    ClientTx {
        new_tx: UnboundedSender<RealtimeMessage>,
        res: Responder<()>,
    },
    GetState {
        res: Responder<ChannelState>,
    },
    GetTx {
        res: Responder<UnboundedSender<RealtimeMessage>>,
    },
    GetTopic {
        res: Responder<String>,
    },
    GetPresenceState {
        res: Responder<PresenceState>,
    },
    PresenceTrack {
        payload: HashMap<String, Value>,
        res: Responder<()>,
    },
    PresenceUntrack {
        res: Responder<()>,
    },
    ReAuth {
        res: Responder<()>,
    },
}

#[derive(Clone, Debug)]
pub struct ChannelManager {
    pub(crate) tx: UnboundedSender<ChannelManagerMessage>,
    rt: Arc<Runtime>,
}

impl ChannelManager {
    pub fn send(
        &self,
        message: ChannelManagerMessage,
    ) -> Result<(), SendError<ChannelManagerMessage>> {
        self.tx.send(message)
    }
    pub fn subscribe(&self) {
        let _ = self.send(ChannelManagerMessage::Subscribe);
    }
    pub async fn unsubscribe(&self) -> Result<Result<ChannelState, ChannelSendError>, RecvError> {
        let (tx, rx) = oneshot::channel();
        let _ = self.send(ChannelManagerMessage::Unsubscribe { res: tx });
        rx.await
    }
    pub async fn subscribe_blocking(&self) -> Result<(), oneshot::error::RecvError> {
        let (tx, rx) = oneshot::channel();
        let _ = self.send(ChannelManagerMessage::SubscribeBlocking { res: tx });
        rx.await
    }
    pub fn broadcast(&self, payload: BroadcastPayload) {
        let _ = self.send(ChannelManagerMessage::Broadcast { payload });
    }
    pub async fn track(&self, payload: HashMap<String, Value>) -> Result<(), RecvError> {
        let (tx, rx) = oneshot::channel();
        let _ = self.send(ChannelManagerMessage::PresenceTrack { payload, res: tx });
        rx.await
    }
    pub async fn untrack(&self) -> Result<(), RecvError> {
        let (tx, rx) = oneshot::channel();
        let _ = self.send(ChannelManagerMessage::PresenceUntrack { res: tx });
        rx.await
    }
    pub async fn reauth(&self) -> Result<(), RecvError> {
        let (tx, rx) = oneshot::channel();
        let _ = self.send(ChannelManagerMessage::ReAuth { res: tx });
        rx.await
    }
    pub async fn get_state(&self) -> Result<ChannelState, RecvError> {
        let (tx, rx) = oneshot::channel();
        let _ = self.send(ChannelManagerMessage::GetState { res: tx });
        rx.await
    }
    pub async fn get_topic(&self) -> String {
        let (tx, rx) = oneshot::channel();
        let _ = self.send(ChannelManagerMessage::GetTopic { res: tx });
        rx.await.unwrap()
    }
    pub async fn get_tx(&self) -> UnboundedSender<RealtimeMessage> {
        let (tx, rx) = oneshot::channel();
        let _ = self.send(ChannelManagerMessage::GetTx { res: tx });
        rx.await.unwrap()
    }
    pub async fn get_presence_state(&self) -> PresenceState {
        let (tx, rx) = oneshot::channel();
        let _ = self.send(ChannelManagerMessage::GetPresenceState { res: tx });
        rx.await.unwrap()
    }
    pub fn to_sync(self) -> ChannelManagerSync {
        ChannelManagerSync { inner: self }
    }
}

#[derive(Clone)]
pub struct ChannelManagerSync {
    inner: ChannelManager,
}

impl ChannelManagerSync {
    pub fn get_rt(&self) -> Arc<Runtime> {
        self.inner.rt.clone()
    }
    pub fn into_inner(self) -> ChannelManager {
        self.inner
    }
    pub fn subscribe(&self) {
        self.inner.subscribe()
    }
    pub fn unsubscribe(&self) -> Result<Result<ChannelState, ChannelSendError>, RecvError> {
        self.inner.rt.block_on(self.inner.unsubscribe())
    }
    pub fn subscribe_blocking(&self) -> Result<(), RecvError> {
        self.inner.rt.block_on(self.inner.subscribe_blocking())
    }
    pub fn broadcast(&self, payload: BroadcastPayload) {
        self.inner.broadcast(payload)
    }
    pub fn get_state(&self) -> Result<ChannelState, RecvError> {
        self.inner.rt.block_on(self.inner.get_state())
    }
    pub fn get_presence_state(&self) -> PresenceState {
        self.inner.rt.block_on(self.inner.get_presence_state())
    }
    pub fn track(&self, payload: HashMap<String, Value>) -> Result<(), RecvError> {
        self.inner.rt.block_on(self.inner.track(payload))
    }
    pub fn untrack(&self) -> Result<(), RecvError> {
        self.inner.rt.block_on(self.inner.untrack())
    }
    pub fn reauth(&self) -> Result<(), RecvError> {
        self.inner.rt.block_on(self.inner.reauth())
    }
}

impl<'a> FromIterator<&'a mut ChannelManager> for Vec<ChannelManager> {
    fn from_iter<T: IntoIterator<Item = &'a mut ChannelManager>>(iter: T) -> Self {
        let mut vec = Vec::new();
        for c in iter {
            vec.push(c.clone());
        }
        vec
    }
}

/// Channel structure
struct RealtimeChannel {
    pub(crate) topic: String,
    pub(crate) state: Arc<Mutex<ChannelState>>,
    pub(crate) id: Uuid,
    pub(crate) cdc_callbacks: Arc<Mutex<HashMap<PostgresChangesEvent, Vec<CdcCallback>>>>,
    pub(crate) broadcast_callbacks: Arc<Mutex<HashMap<String, Vec<BroadcastCallback>>>>,
    pub(crate) client_tx: mpsc::UnboundedSender<RealtimeMessage>,
    join_payload: JoinPayload,
    presence: Arc<Mutex<RealtimePresence>>,
    pub(crate) tx: Option<UnboundedSender<RealtimeMessage>>,
    pub(crate) manager_channel: (
        UnboundedSender<ChannelManagerMessage>,
        UnboundedReceiver<ChannelManagerMessage>,
    ),
    pub(crate) message_handle: Option<JoinHandle<()>>,
    rt: Arc<Runtime>,
    access_token: Arc<Mutex<String>>,
}

impl RealtimeChannel {
    async fn manager_recv(&mut self) {
        while let Some(control_message) = self.manager_channel.1.recv().await {
            match control_message {
                ChannelManagerMessage::Subscribe => {
                    self.subscribe().await;
                }
                ChannelManagerMessage::Unsubscribe { res } => {
                    res.send(self.unsubscribe().await).unwrap();
                }
                ChannelManagerMessage::SubscribeBlocking { res } => {
                    self.subscribe_blocking(res).await;
                }
                ChannelManagerMessage::Broadcast { payload } => {
                    self.broadcast(payload).await.unwrap();
                }
                ChannelManagerMessage::ClientTx { new_tx, res } => {
                    self.client_tx = new_tx;
                    res.send(()).unwrap();
                }
                ChannelManagerMessage::GetState { res } => {
                    res.send(self.state.lock().await.clone()).unwrap();
                }
                ChannelManagerMessage::GetTx { res } => {
                    res.send(self.tx.clone().unwrap()).unwrap();
                }
                ChannelManagerMessage::GetTopic { res } => {
                    res.send(self.topic.clone()).unwrap();
                }
                ChannelManagerMessage::PresenceTrack { payload, res } => {
                    res.send(self.track(payload).await.unwrap()).unwrap()
                }
                ChannelManagerMessage::PresenceUntrack { res } => {
                    res.send(self.untrack().await.unwrap()).unwrap()
                }
                ChannelManagerMessage::GetPresenceState { res } => {
                    let presence = self.presence.lock().await;
                    res.send(presence.state.clone()).unwrap();
                }
                ChannelManagerMessage::ReAuth { res } => {
                    res.send(self.reauth().await.unwrap()).unwrap();
                } // TODO kill message
            }
        }
    }

    /// Send a join request to the channel
    async fn subscribe(&mut self) {
        let join_message = RealtimeMessage {
            event: MessageEvent::PhxJoin,
            topic: self.topic.clone(),
            payload: Payload::Join(self.join_payload.clone()),
            message_ref: Some(self.id.into()),
        };

        let mut state = self.state.lock().await;
        *state = ChannelState::Joining;
        drop(state);

        let _ = self.send(join_message.into()).await;
    }

    async fn subscribe_blocking(&mut self, tx: Responder<()>) {
        self.subscribe().await;

        let state = self.state.clone();

        self.rt.spawn(async move {
            loop {
                let state = state.lock().await;
                if *state == ChannelState::Joined {
                    break;
                }
            }
            tx.send(()).unwrap();
        });
    }

    fn client_recv(&mut self) {
        let (channel_tx, mut channel_rx) = mpsc::unbounded_channel::<RealtimeMessage>();
        self.tx = Some(channel_tx);
        let task_state = self.state.clone();
        let task_cdc_cbs = self.cdc_callbacks.clone();
        let task_bc_cbs = self.broadcast_callbacks.clone();
        let id = self.id;
        let presence = self.presence.clone();

        self.message_handle = Some(self.rt.spawn(async move {
            while let Some(message) = channel_rx.recv().await {
                // get locks
                let mut broadcast_callbacks = task_bc_cbs.lock().await;
                let mut cdc_callbacks = task_cdc_cbs.lock().await;

                let test_message = message.clone(); // TODO fix dis

                match message.payload {
                    Payload::Broadcast(payload) => {
                        if let Some(cb_vec) = broadcast_callbacks.get_mut(&payload.event) {
                            for cb in cb_vec {
                                cb(&payload.payload);
                            }
                        }
                    }
                    Payload::PostgresChanges(payload) => {
                        if let Some(cb_vec) = cdc_callbacks.get_mut(&payload.data.change_type) {
                            for cb in cb_vec {
                                if cb.0.check(test_message.clone()).is_none() {
                                    continue;
                                }
                                cb.1(&payload);
                            }
                        }
                        if let Some(cb_vec) = cdc_callbacks.get_mut(&PostgresChangesEvent::All) {
                            for cb in cb_vec {
                                if cb.0.check(test_message.clone()).is_none() {
                                    continue;
                                }
                                cb.1(&payload);
                            }
                        }
                    }
                    Payload::Response(join_response) => {
                        let target_id = message.message_ref.clone().unwrap_or("".to_string());
                        if target_id != id.to_string() {
                            return;
                        }
                        if join_response.status == PayloadStatus::Ok {
                            let mut channel_state = task_state.lock().await;
                            *channel_state = ChannelState::Joined;
                            drop(channel_state);
                        }
                    }
                    Payload::PresenceDiff(diff) => {
                        let mut presence = presence.lock().await;
                        presence.sync_diff(diff.into());
                    }
                    Payload::PresenceState(state) => {
                        let mut presence = presence.lock().await;
                        presence.sync(state.into());
                    }
                    _ => {
                        println!("Unmatched payload ;_;")
                    }
                }

                drop(broadcast_callbacks);
                drop(cdc_callbacks);
            }
        }));
    }

    /// Leave the channel
    async fn unsubscribe(&mut self) -> Result<ChannelState, ChannelSendError> {
        let state = self.state.clone();
        {
            let state = state.lock().await;
            if *state == ChannelState::Closed || *state == ChannelState::Leaving {
                let s = state.clone();
                return Ok(s);
            }
        }

        match self
            .send(RealtimeMessage {
                event: MessageEvent::PhxLeave,
                topic: self.topic.clone(),
                payload: Payload::Empty {},
                message_ref: Some(format!("{}+leave", self.id)),
            })
            .await
        {
            Ok(()) => {
                let mut state = state.lock().await;
                *state = ChannelState::Leaving;
                Ok(*state)
            }
            Err(ChannelSendError::ChannelError(status)) => Ok(status),
            Err(e) => Err(e),
        }
    }

    /// Track provided state in Realtime Presence
    async fn track(&mut self, payload: HashMap<String, Value>) -> Result<(), ChannelSendError> {
        self.send(RealtimeMessage {
            event: MessageEvent::Presence,
            topic: self.topic.clone(),
            payload: Payload::PresenceTrack(payload.into()),
            message_ref: None,
        })
        .await
    }

    /// Sends a message to stop tracking this channel's presence
    async fn untrack(&mut self) -> Result<(), ChannelSendError> {
        self.send(RealtimeMessage {
            event: MessageEvent::Untrack,
            topic: self.topic.clone(),
            payload: Payload::Empty {},
            message_ref: None,
        })
        .await
    }

    async fn send(&mut self, message: RealtimeMessage) -> Result<(), ChannelSendError> {
        // inject channel topic to message here
        let mut message = message.clone();
        message.topic = self.topic.clone();

        let state = self.state.lock().await;

        if *state == ChannelState::Leaving {
            return Err(ChannelSendError::ChannelError(state.clone()));
        }

        match self.client_tx.send(message.into()) {
            Ok(()) => Ok(()),
            Err(e) => Err(ChannelSendError::SendError(e)),
        }
    }

    async fn broadcast(&mut self, payload: BroadcastPayload) -> Result<(), ChannelSendError> {
        self.send(RealtimeMessage {
            event: MessageEvent::Broadcast,
            topic: "".into(),
            payload: Payload::Broadcast(payload),
            message_ref: None,
        })
        .await
    }

    async fn reauth(&mut self) -> Result<(), ChannelSendError> {
        // TODO test this
        let access_token = self.access_token.lock().await;

        self.join_payload.access_token = access_token.clone();

        let state = self.state.lock().await;

        if *state != ChannelState::Joined {
            return Ok(());
        }

        drop(state);

        let access_token_message = RealtimeMessage {
            event: MessageEvent::AccessToken,
            topic: self.topic.clone(),
            payload: Payload::AccessToken(AccessTokenPayload {
                access_token: access_token.clone(),
            }),
            ..Default::default()
        };

        drop(access_token);

        self.send(access_token_message).await
    }
}

/// Builder struct for [RealtimeChannel]
///
/// Get access to this through [RealtimeClient::channel()]
pub struct RealtimeChannelBuilder {
    topic: String,
    broadcast: BroadcastConfig,
    presence: PresenceConfig,
    id: Uuid,
    postgres_changes: Vec<PostgresChange>,
    cdc_callbacks: HashMap<PostgresChangesEvent, Vec<CdcCallback>>,
    broadcast_callbacks: HashMap<String, Vec<BroadcastCallback>>,
    presence_callbacks: PresenceCallbackMap,
}

impl RealtimeChannelBuilder {
    pub fn new(topic: impl Into<String>) -> Self {
        Self {
            topic: format!("realtime:{}", topic.into()),
            broadcast: Default::default(),
            presence: Default::default(),
            id: Uuid::new_v4(),
            postgres_changes: Default::default(),
            cdc_callbacks: Default::default(),
            broadcast_callbacks: Default::default(),
            presence_callbacks: Default::default(),
        }
    }

    /// Set the topic of the channel
    pub fn topic(mut self, topic: impl Into<String>) -> Self {
        self.topic = format!("realtime:{}", topic.into());
        self
    }

    /// Set the broadcast config for this channel
    pub fn broadcast(mut self, broadcast_config: BroadcastConfig) -> Self {
        self.broadcast = broadcast_config;
        self
    }

    /// Set the presence config for this channel
    pub fn presence(mut self, presence_config: PresenceConfig) -> Self {
        self.presence = presence_config;
        self
    }

    /// Add a postgres changes callback to this channel
    ///```
    /// TODO code
    pub fn on_postgres_change(
        mut self,
        event: PostgresChangesEvent,
        filter: PostgresChangeFilter,
        callback: impl FnMut(&PostgresChangesPayload) + 'static + Send,
    ) -> Self {
        self.postgres_changes.push(PostgresChange {
            event: event.clone(),
            schema: filter.schema.clone(),
            table: filter.table.clone().unwrap_or("".into()),
            filter: filter.filter.clone(),
        });

        if self.cdc_callbacks.get_mut(&event).is_none() {
            self.cdc_callbacks.insert(event.clone(), vec![]);
        }

        self.cdc_callbacks
            .get_mut(&event)
            .unwrap_or(&mut vec![])
            .push((filter, Box::new(callback)));

        self
    }

    /// Add a presence callback to this channel
    ///```
    /// TODO code
    pub fn on_presence(
        mut self,
        event: PresenceEvent,
        // TODO callback type alias
        callback: impl Fn(String, PresenceState, PresenceState) + Send + 'static,
    ) -> Self {
        if self.presence_callbacks.get_mut(&event).is_none() {
            self.presence_callbacks.insert(event.clone(), vec![]);
        }

        self.presence_callbacks
            .get_mut(&event)
            .unwrap_or(&mut vec![])
            .push(Box::new(callback));

        self
    }

    /// Add a broadcast callback to this channel
    /// ```
    /// TODO code
    pub fn on_broadcast(
        mut self,
        event: impl Into<String>,
        callback: impl FnMut(&HashMap<String, Value>) + 'static + Send,
    ) -> Self {
        let event: String = event.into();

        if self.broadcast_callbacks.get_mut(&event).is_none() {
            self.broadcast_callbacks.insert(event.clone(), vec![]);
        }

        self.broadcast_callbacks
            .get_mut(&event)
            .unwrap_or(&mut vec![])
            .push(Box::new(callback));

        self
    }

    // TODO on_message handler for sys messages
    //
    fn build_common(
        self,
        client_tx: UnboundedSender<RealtimeMessage>,
        access_token: String,
        access_token_arc: Arc<Mutex<String>>,
        rt: Arc<Runtime>,
    ) -> ChannelManager {
        let state = Arc::new(Mutex::new(ChannelState::Closed));
        let cdc_callbacks = Arc::new(Mutex::new(self.cdc_callbacks));
        let broadcast_callbacks = Arc::new(Mutex::new(self.broadcast_callbacks));
        let (controller_tx, controller_rx) = mpsc::unbounded_channel::<ChannelManagerMessage>();

        let mut channel = RealtimeChannel {
            access_token: access_token_arc,
            rt: rt.clone(),
            tx: None,
            topic: self.topic,
            cdc_callbacks,
            broadcast_callbacks,
            client_tx,
            state,
            id: self.id,
            join_payload: JoinPayload {
                config: JoinConfig {
                    broadcast: self.broadcast,
                    presence: self.presence,
                    postgres_changes: self.postgres_changes,
                },
                access_token,
            },
            presence: Arc::new(Mutex::new(RealtimePresence::from_channel_builder(
                self.presence_callbacks,
            ))),
            manager_channel: (controller_tx, controller_rx),
            message_handle: None,
        };

        channel.client_recv(); // Spawns task, sets channel.tx
        let tx = channel.manager_channel.0.clone();

        let _handle = rt.spawn(async move { channel.manager_recv().await });

        let channel_manager = ChannelManager { tx, rt };

        channel_manager
    }

    pub fn build_sync(self, client: &ClientManagerSync) -> Result<ChannelManagerSync, RecvError> {
        let client_tx = client.clone().get_ws_tx().unwrap();
        let access_token = client.clone().get_access_token().unwrap();
        let access_token_arc = client.clone().get_access_token_arc().unwrap();

        let channel_manager =
            self.build_common(client_tx, access_token, access_token_arc, client.get_rt());

        client.add_channel(channel_manager.clone()).unwrap();

        Ok(channel_manager.to_sync())
    }

    pub async fn build(self, client: &ClientManager) -> Result<ChannelManager, RecvError> {
        let client_tx = client.clone().get_ws_tx().await?;
        let access_token = client.clone().get_access_token().await?;
        let access_token_arc = client.clone().get_access_token_arc().await?;

        let channel_manager =
            self.build_common(client_tx, access_token, access_token_arc, client.get_rt());

        client.add_channel(channel_manager.clone()).await.unwrap();

        Ok(channel_manager)
    }
}
