use serde_json::Value;
use tokio::{
    sync::{
        mpsc::{self, error::SendError, UnboundedReceiver, UnboundedSender},
        oneshot::{self, error::RecvError},
        Mutex,
    },
    task::JoinHandle,
};
use tokio_tungstenite::tungstenite::Message;
use uuid::Uuid;

use crate::message::{
    payload::{
        AccessTokenPayload, BroadcastConfig, BroadcastPayload, JoinConfig, JoinPayload, Payload,
        PayloadStatus, PostgresChange, PostgresChangesEvent, PostgresChangesPayload,
        PresenceConfig,
    },
    presence::{PresenceCallback, PresenceEvent, PresenceState},
    MessageEvent, PostgresChangeFilter, RealtimeMessage,
};

use crate::sync::realtime_presence::RealtimePresence;
use std::fmt::Debug;
use std::{collections::HashMap, sync::Arc};

use super::{ClientManager, Responder};

type CdcCallback = (
    PostgresChangeFilter,
    Box<dyn FnMut(&PostgresChangesPayload) + Send>,
);
type BroadcastCallback = Box<dyn FnMut(&HashMap<String, Value>) + Send>;

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
    SendError(SendError<Message>),
    ChannelError(ChannelState),
}

pub enum ChannelManagerMessage {
    Subscribe,
    SubscribeBlocking {
        res: Responder<()>,
    },
    Broadcast {
        payload: BroadcastPayload,
    },
    ClientTx {
        new_tx: UnboundedSender<Message>,
        res: Responder<()>,
    },
    State {
        res: Responder<ChannelState>,
    },
    GetTx {
        res: Responder<UnboundedSender<Message>>,
    },
    GetTopic {
        res: Responder<String>,
    },
}

#[derive(Clone)]
pub struct ChannelManager {
    pub(crate) tx: UnboundedSender<ChannelManagerMessage>,
}

// TODO rethink where channel state is kept. Maybe keep it on the client?

impl ChannelManager {
    pub fn oneshot<T>() -> (oneshot::Sender<T>, oneshot::Receiver<T>) {
        // TODO does this really save enough keystrokes? maybe abstracted early here
        oneshot::channel::<T>()
    }
    pub fn send(
        &self,
        message: ChannelManagerMessage,
    ) -> Result<(), SendError<ChannelManagerMessage>> {
        self.tx.send(message)
    }
    pub fn subscribe(&self) {
        let _ = self.send(ChannelManagerMessage::Subscribe);
    }
    pub async fn subscribe_blocking(&self) -> Result<(), oneshot::error::RecvError> {
        let (tx, rx) = Self::oneshot();
        let _ = self.send(ChannelManagerMessage::SubscribeBlocking { res: tx });
        rx.await
    }
    pub fn broadcast(&self, payload: BroadcastPayload) {
        let _ = self.send(ChannelManagerMessage::Broadcast { payload });
    }
    pub async fn state(&self) -> ChannelState {
        let (tx, rx) = Self::oneshot::<ChannelState>();
        let _ = self.send(ChannelManagerMessage::State { res: tx });
        rx.await.unwrap()
    }
    pub async fn get_topic(&self) -> String {
        let (tx, rx) = oneshot::channel();
        let _ = self.send(ChannelManagerMessage::GetTopic { res: tx });
        rx.await.unwrap()
    }
    pub async fn get_tx(&self) -> UnboundedSender<Message> {
        let (tx, rx) = oneshot::channel();
        let _ = self.send(ChannelManagerMessage::GetTx { res: tx });
        rx.await.unwrap()
    }
}

/// Channel structure
pub struct RealtimeChannel {
    pub(crate) topic: String,
    pub(crate) state: Arc<Mutex<ChannelState>>,
    pub(crate) id: Uuid,
    pub(crate) cdc_callbacks: Arc<Mutex<HashMap<PostgresChangesEvent, Vec<CdcCallback>>>>,
    pub(crate) broadcast_callbacks: Arc<Mutex<HashMap<String, Vec<BroadcastCallback>>>>,
    pub(crate) client_tx: mpsc::UnboundedSender<Message>,
    join_payload: JoinPayload,
    presence: RealtimePresence,
    pub(crate) tx: Option<UnboundedSender<Message>>,
    pub manager_channel: (
        UnboundedSender<ChannelManagerMessage>,
        UnboundedReceiver<ChannelManagerMessage>,
    ),
    pub(crate) message_handle: Option<JoinHandle<()>>,
}

impl RealtimeChannel {
    async fn manager_recv(&mut self) {
        while let Some(control_message) = self.manager_channel.1.recv().await {
            match control_message {
                ChannelManagerMessage::Subscribe => {
                    self.subscribe().await;
                }
                ChannelManagerMessage::SubscribeBlocking { res } => {
                    let _ = res.send(self.subscribe_blocking().await);
                }
                ChannelManagerMessage::Broadcast { payload } => {
                    let _ = self.broadcast(payload).await;
                }
                ChannelManagerMessage::ClientTx { new_tx, res } => {
                    self.client_tx = new_tx;
                    let _ = res.send(());
                }
                ChannelManagerMessage::State { res } => {
                    let _ = res.send(self.state.lock().await.clone());
                }
                ChannelManagerMessage::GetTx { res } => {
                    let _ = res.send(self.tx.clone().unwrap());
                }
                ChannelManagerMessage::GetTopic { res } => {
                    let _ = res.send(self.topic.clone());
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

    async fn subscribe_blocking(&mut self) {
        self.subscribe().await;

        loop {
            let state = self.state.lock().await;
            if *state == ChannelState::Joined {
                break;
            }
        }
    }

    fn client_recv(&mut self) {
        let (channel_tx, mut channel_rx) = mpsc::unbounded_channel::<Message>();
        self.tx = Some(channel_tx);
        let task_state = self.state.clone();
        let task_cdc_cbs = self.cdc_callbacks.clone();
        let task_bc_cbs = self.broadcast_callbacks.clone();
        let id = self.id;

        self.message_handle = Some(tokio::spawn(async move {
            while let Some(message) = channel_rx.recv().await {
                let message: RealtimeMessage =
                    serde_json::from_str(message.to_text().unwrap()).unwrap();

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
        let mut state = state.lock().await;
        if *state == ChannelState::Closed || *state == ChannelState::Leaving {
            let s = state.clone();
            return Ok(s);
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
                *state = ChannelState::Leaving;
                Ok(*state)
            }
            Err(ChannelSendError::ChannelError(status)) => Ok(status),
            Err(e) => Err(e),
        }
    }

    /// Returns the current [PresenceState] of the channel
    fn presence_state(&self) -> PresenceState {
        self.presence.state.clone()
    }

    /// Track provided state in Realtime Presence
    /// ```
    /// # use std::{collections::HashMap, env};
    /// # use realtime_rs::sync::*;
    /// # use realtime_rs::message::*;  
    /// # use realtime_rs::*;          
    /// # fn main() -> Result<(), ()> {
    /// #   let url = "http://127.0.0.1:54321";
    /// #   let anon_key = env::var("LOCAL_ANON_KEY").expect("No anon key!");
    /// #
    /// #   let mut client = RealtimeClient::builder(url, anon_key)
    /// #       .build();
    /// #
    /// #   match client.connect() {
    /// #       Ok(_) => {}
    /// #       Err(e) => panic!("Couldn't connect! {:?}", e),
    /// #   };
    /// #
    /// #   let channel_id = client.channel("topic").build(&mut client);
    /// #
    /// #   let _ = client.block_until_subscribed(channel_id);
    /// #
    ///     client
    ///         .get_channel_mut(channel_id)
    ///         .unwrap()
    ///         .track(HashMap::new());
    /// #   Ok(())
    /// #   }
    fn track(&mut self, payload: HashMap<String, Value>) -> &mut RealtimeChannel {
        let _ = self.send(RealtimeMessage {
            event: MessageEvent::Presence,
            topic: self.topic.clone(),
            payload: Payload::PresenceTrack(payload.into()),
            message_ref: None,
        });

        self
    }

    /// Sends a message to stop tracking this channel's presence
    fn untrack(&mut self) {
        let _ = self.send(RealtimeMessage {
            event: MessageEvent::Untrack,
            topic: self.topic.clone(),
            payload: Payload::Empty {},
            message_ref: None,
        });
    }

    /// Send a [RealtimeMessage] on this channel
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

    /// Helper function for sending broadcast messages
    ///```
    ///TODO CODE
    async fn broadcast(&mut self, payload: BroadcastPayload) -> Result<(), ChannelSendError> {
        self.send(RealtimeMessage {
            event: MessageEvent::Broadcast,
            topic: "".into(),
            payload: Payload::Broadcast(payload),
            message_ref: None,
        })
        .await
    }

    pub(crate) async fn set_auth(&mut self, access_token: String) -> Result<(), ChannelSendError> {
        self.join_payload.access_token = access_token.clone();

        let state = self.state.lock().await;

        if *state != ChannelState::Joined {
            return Ok(());
        }

        drop(state);

        let access_token_message = RealtimeMessage {
            event: MessageEvent::AccessToken,
            topic: self.topic.clone(),
            payload: Payload::AccessToken(AccessTokenPayload { access_token }),
            ..Default::default()
        };

        self.send(access_token_message).await
    }

    // pub(crate) fn recieve(&mut self, message: RealtimeMessage) {
    //     match &message.payload {
    //         Payload::Response(join_response) => {
    //             let target_id = message.message_ref.clone().unwrap_or("".to_string());
    //             if target_id != self.id.to_string() {
    //                 return;
    //             }
    //             if join_response.status == PayloadStatus::Ok {
    //                 self.status = ChannelState::Joined;
    //             }
    //         }
    //         Payload::PresenceState(state) => self.presence.sync(state.clone().into()),
    //         Payload::PresenceDiff(raw_diff) => {
    //             self.presence.sync_diff(raw_diff.clone().into());
    //         }
    //         Payload::PostgresChanges(payload) => {
    //             let event = &payload.data.change_type;
    //
    //             for cdc_callback in self.cdc_callbacks.get_mut(event).unwrap_or(&mut vec![]) {
    //                 let filter = &cdc_callback.0;
    //
    //                 // TODO REFAC pointless message clones when not using result; filter.check
    //                 // should borrow and return bool/result
    //                 if let Some(_message) = filter.check(message.clone()) {
    //                     cdc_callback.1(payload);
    //                 }
    //             }
    //
    //             for cdc_callback in self
    //                 .cdc_callbacks
    //                 .get_mut(&PostgresChangesEvent::All)
    //                 .unwrap_or(&mut vec![])
    //             {
    //                 let filter = &cdc_callback.0;
    //
    //                 if let Some(_message) = filter.check(message.clone()) {
    //                     cdc_callback.1(payload);
    //                 }
    //             }
    //         }
    //         Payload::Broadcast(payload) => {
    //             if let Some(callbacks) = self.broadcast_callbacks.get_mut(&payload.event) {
    //                 for cb in callbacks {
    //                     cb(&payload.payload);
    //                 }
    //             }
    //         }
    //         _ => {}
    //     }
    //
    //     match &message.event {
    //         MessageEvent::PhxClose => {
    //             if let Some(message_ref) = message.message_ref {
    //                 if message_ref == self.id.to_string() {
    //                     self.status = ChannelState::Closed;
    //                     if DEBUG {
    //                         println!("Channel Closed! {:?}", self.id);
    //                     }
    //                 }
    //             }
    //         }
    //         MessageEvent::PhxReply => {
    //             if message.message_ref.clone().unwrap_or("#NOREF".to_string())
    //                 == format!("{}+leave", self.id)
    //             {
    //                 self.status = ChannelState::Closed;
    //                 if DEBUG {
    //                     println!("Channel Closed! {:?}", self.id);
    //                 }
    //             }
    //         }
    //         _ => {}
    //     }
    // }
}

impl Debug for RealtimeChannel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&format!(
            "RealtimeChannel {{ name: {:?}, callbacks: [TODO DEBUG]}}",
            self.topic
        ))
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
    presence_callbacks: HashMap<PresenceEvent, Vec<PresenceCallback>>,
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
    /// # use realtime_rs::sync::*;
    /// # use realtime_rs::message::*;  
    /// # use realtime_rs::message::payload::*;  
    /// # use realtime_rs::*;          
    /// # use std::env;
    /// #
    /// # fn main() -> Result<(), ()> {
    /// #     let url = "http://127.0.0.1:54321";
    /// #     let anon_key = env::var("LOCAL_ANON_KEY").expect("No anon key!");
    /// #     let mut client = RealtimeClient::builder(url, anon_key).build();
    /// #     let _ = client.connect();
    ///
    ///     let my_pgc_callback = move |msg: &_| {
    ///         println!("Got message: {:?}", msg);
    ///     };
    ///
    ///     let channel_id = client
    ///         .channel("topic")
    ///         .on_postgres_change(
    ///             PostgresChangesEvent::All,
    ///             PostgresChangeFilter {
    ///                 schema: "public".into(),
    ///                 table: Some("todos".into()),
    ///                 ..Default::default()
    ///             },
    ///             my_pgc_callback,
    ///         )
    ///         .build(&mut client);
    /// #
    /// #     client.get_channel_mut(channel_id).unwrap().subscribe();
    /// #     loop {
    /// #         if client.get_status() == ConnectionState::Closed {
    /// #             break;
    /// #         }
    /// #         match client.next_message() {
    /// #             Ok(_topic) => return Ok(()),
    /// #             Err(NextMessageError::WouldBlock) => return Ok(()),
    /// #             Err(_e) => return Err(()),
    /// #         }
    /// #     }
    /// #     Err(())
    /// # }
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
    /// # use realtime_rs::sync::*;
    /// # use realtime_rs::message::*;  
    /// # use realtime_rs::*;          
    /// # use std::env;
    /// #
    /// # fn main() -> Result<(), ()> {
    /// #     let url = "http://127.0.0.1:54321";
    /// #     let anon_key = env::var("LOCAL_ANON_KEY").expect("No anon key!");
    /// #     let mut client = RealtimeClient::builder(url, anon_key).build();
    /// #     let _ = client.connect();
    ///
    ///     let channel_id = client
    ///         .channel("topic".to_string())
    ///         .on_presence(PresenceEvent::Sync, |key, old_state, new_state| {
    ///             println!("Presence sync: {:?}, {:?}, {:?}", key, old_state, new_state);
    ///         })
    ///         .build(&mut client);
    ///
    /// #     client.get_channel_mut(channel_id).unwrap().subscribe();
    /// #     loop {
    /// #         if client.get_status() == ConnectionState::Closed {
    /// #             break;
    /// #         }
    /// #         match client.next_message() {
    /// #             Ok(_topic) => return Ok(()),
    /// #             Err(NextMessageError::WouldBlock) => return Ok(()),
    /// #             Err(_e) => return Err(()),
    /// #         }
    /// #     }
    /// #     Err(())
    /// # }
    pub fn on_presence(
        mut self,
        event: PresenceEvent,
        // TODO callback type alias
        callback: impl FnMut(String, PresenceState, PresenceState) + 'static + Send,
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
    /// # use realtime_rs::sync::*;
    /// # use realtime_rs::message::*;  
    /// # use realtime_rs::*;          
    /// # use std::env;
    /// #
    /// # fn main() -> Result<(), ()> {
    /// #     let url = "http://127.0.0.1:54321";
    /// #     let anon_key = env::var("LOCAL_ANON_KEY").expect("No anon key!");
    /// #     let mut client = RealtimeClient::builder(url, anon_key).build();
    /// #     let _ = client.connect();
    ///
    ///     let channel_id = client
    ///         .channel("topic")
    ///         .on_broadcast("subtopic", |msg| {
    ///             println!("recieved broadcast: {:?}", msg);
    ///         })
    ///         .build(&mut client);
    ///
    /// #     client.get_channel_mut(channel_id).unwrap().subscribe();
    /// #     loop {
    /// #         if client.get_status() == ConnectionState::Closed {
    /// #             break;
    /// #         }
    /// #         match client.next_message() {
    /// #             Ok(_topic) => return Ok(()),
    /// #             Err(NextMessageError::WouldBlock) => return Ok(()),
    /// #             Err(_e) => return Err(()),
    /// #         }
    /// #     }
    /// #     Err(())
    /// # }
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

    /// Create the channel and pass ownership to provided [RealtimeClient], returning the channel
    /// id for later access through the client
    pub async fn build(self, client: ClientManager) -> Result<ChannelManager, RecvError> {
        let state = Arc::new(Mutex::new(ChannelState::Closed));
        let cdc_callbacks = Arc::new(Mutex::new(self.cdc_callbacks));
        let broadcast_callbacks = Arc::new(Mutex::new(self.broadcast_callbacks));
        let (controller_tx, controller_rx) = mpsc::unbounded_channel::<ChannelManagerMessage>();

        let client_tx = client.clone().get_ws_tx().await?;
        let access_token = client.clone().get_access_token().await?;

        let mut channel = RealtimeChannel {
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
            presence: RealtimePresence::from_channel_builder(self.presence_callbacks),
            manager_channel: (controller_tx, controller_rx),
            message_handle: None,
        };

        channel.client_recv(); // Spawns task, sets channel.tx

        let tx = channel.manager_channel.0.clone();

        let _handle = tokio::spawn(async move { channel.manager_recv().await });

        let channel_manager = ChannelManager { tx };

        client.add_channel(channel_manager.clone()).await.unwrap();

        Ok(channel_manager)
    }
}
