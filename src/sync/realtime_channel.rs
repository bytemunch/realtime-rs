use futures_channel::mpsc::TrySendError;
use serde_json::Value;
use tokio_tungstenite::tungstenite::Message;
use uuid::Uuid;

use crate::{
    message::{
        payload::{
            AccessTokenPayload, BroadcastConfig, BroadcastPayload, JoinConfig, JoinPayload,
            Payload, PayloadStatus, PostgresChange, PostgresChangesEvent, PostgresChangesPayload,
            PresenceConfig,
        },
        presence::{PresenceCallback, PresenceEvent, PresenceState},
        MessageEvent, PostgresChangeFilter, RealtimeMessage,
    },
    DEBUG,
};

use crate::sync::{realtime_client::RealtimeClient, realtime_presence::RealtimePresence};
use std::collections::HashMap;
use std::fmt::Debug;

type CdcCallback = (
    PostgresChangeFilter,
    Box<dyn FnMut(&PostgresChangesPayload)>,
);
type BroadcastCallback = Box<dyn FnMut(&HashMap<String, Value>)>;

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
    SendError(TrySendError<Message>),
    ChannelError(ChannelState),
}

/// Channel structure
pub struct RealtimeChannel {
    pub(crate) topic: String,
    pub(crate) status: ChannelState,
    pub(crate) id: Uuid,
    cdc_callbacks: HashMap<PostgresChangesEvent, Vec<CdcCallback>>,
    broadcast_callbacks: HashMap<String, Vec<BroadcastCallback>>,
    client_tx: futures_channel::mpsc::UnboundedSender<Message>,
    pub(crate) channel_tx: futures_channel::mpsc::UnboundedSender<Message>,
    channel_rx: futures_channel::mpsc::UnboundedReceiver<Message>,
    join_payload: JoinPayload,
    presence: RealtimePresence,
}

// TODO channel options with broadcast + presence settings

impl RealtimeChannel {
    /// Returns the channel's connection state
    pub fn get_status(&self) -> ChannelState {
        self.status
    }

    /// Send a join request to the channel
    /// Does not block, for blocking behaviour use [RealtimeClient::block_until_subscribed()]
    pub fn subscribe(&mut self) {
        let join_message = RealtimeMessage {
            event: MessageEvent::PhxJoin,
            topic: self.topic.clone(),
            payload: Payload::Join(self.join_payload.clone()),
            message_ref: Some(self.id.into()),
        };

        self.status = ChannelState::Joining;

        let _ = self.client_tx.unbounded_send(join_message.into());
    }

    /// Leave the channel
    pub fn unsubscribe(&mut self) -> Result<ChannelState, ChannelSendError> {
        if self.status == ChannelState::Closed || self.status == ChannelState::Leaving {
            return Ok(self.status);
        }

        match self.send(RealtimeMessage {
            event: MessageEvent::PhxLeave,
            topic: self.topic.clone(),
            payload: Payload::Empty {},
            message_ref: Some(format!("{}+leave", self.id)),
        }) {
            Ok(()) => {
                self.status = ChannelState::Leaving;
                Ok(self.status)
            }
            Err(ChannelSendError::ChannelError(status)) => Ok(status),
            Err(e) => Err(e),
        }
    }

    /// Returns the current [PresenceState] of the channel
    pub fn presence_state(&self) -> PresenceState {
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
    pub fn track(&mut self, payload: HashMap<String, Value>) -> &mut RealtimeChannel {
        let _ = self.send(RealtimeMessage {
            event: MessageEvent::Presence,
            topic: self.topic.clone(),
            payload: Payload::PresenceTrack(payload.into()),
            message_ref: None,
        });

        self
    }

    /// Sends a message to stop tracking this channel's presence
    pub fn untrack(&mut self) {
        let _ = self.send(RealtimeMessage {
            event: MessageEvent::Untrack,
            topic: self.topic.clone(),
            payload: Payload::Empty {},
            message_ref: None,
        });
    }

    /// Send a [RealtimeMessage] on this channel
    pub fn send(&mut self, message: RealtimeMessage) -> Result<(), ChannelSendError> {
        // inject channel topic to message here
        let mut message = message.clone();
        message.topic = self.topic.clone();

        if self.status == ChannelState::Leaving {
            return Err(ChannelSendError::ChannelError(self.status));
        }

        match self.client_tx.unbounded_send(message.into()) {
            Ok(()) => Ok(()),
            Err(e) => Err(ChannelSendError::SendError(e)),
        }
    }

    /// Helper function for sending broadcast messages
    ///```
    /// # use std::{collections::HashMap, env};
    /// # use realtime_rs::sync::*;
    /// # use realtime_rs::message::*;  
    /// # use realtime_rs::*;          
    /// # use realtime_rs::message::payload::*;  
    /// # fn main() -> Result<(), ()> {
    /// #   let url = "http://127.0.0.1:54321";
    /// #   let anon_key = env::var("LOCAL_ANON_KEY").expect("No anon key!");
    /// #   let mut client = RealtimeClient::builder(url, anon_key).build();
    /// #   let _ = client.connect();
    /// #   let channel = client // TODO broadcast self true
    /// #       .channel("topic")
    /// #       .build(&mut client);
    /// #
    /// #  let _ = client.block_until_subscribed(channel).unwrap();
    /// #
    ///    let mut payload = HashMap::new();
    ///    payload.insert("message".into(), "hello, broadcast!".into());
    ///
    ///    let message = BroadcastPayload::new("event", payload);
    ///
    ///    let _ = client.get_channel_mut(channel).unwrap().broadcast(message);
    /// #
    /// #   loop {
    /// #       match client.next_message() {
    /// #           Ok(_) => {
    /// #               return Ok(()); // TODO test: return OK when we recieve the broadcast
    /// #           }
    /// #           Err(NextMessageError::WouldBlock) => {}
    /// #           Err(_e) => {
    /// #               return Err(());
    /// #           }
    /// #       }
    /// #   }
    /// # }
    pub fn broadcast(&mut self, payload: BroadcastPayload) -> Result<(), ChannelSendError> {
        self.send(RealtimeMessage {
            event: MessageEvent::Broadcast,
            topic: "".into(),
            payload: Payload::Broadcast(payload),
            message_ref: None,
        })
    }

    pub(crate) fn set_auth(&mut self, access_token: String) -> Result<(), ChannelSendError> {
        self.join_payload.access_token = access_token.clone();

        if self.status != ChannelState::Joined {
            return Ok(());
        }

        let access_token_message = RealtimeMessage {
            event: MessageEvent::AccessToken,
            topic: self.topic.clone(),
            payload: Payload::AccessToken(AccessTokenPayload { access_token }),
            ..Default::default()
        };

        self.send(access_token_message)
    }

    pub(crate) fn recieve(&mut self, message: RealtimeMessage) {
        match &message.payload {
            Payload::Response(join_response) => {
                let target_id = message.message_ref.clone().unwrap_or("".to_string());
                if target_id != self.id.to_string() {
                    return;
                }
                if join_response.status == PayloadStatus::Ok {
                    self.status = ChannelState::Joined;
                }
            }
            Payload::PresenceState(state) => self.presence.sync(state.clone().into()),
            Payload::PresenceDiff(raw_diff) => {
                self.presence.sync_diff(raw_diff.clone().into());
            }
            Payload::PostgresChanges(payload) => {
                let event = &payload.data.change_type;

                for cdc_callback in self.cdc_callbacks.get_mut(event).unwrap_or(&mut vec![]) {
                    let filter = &cdc_callback.0;

                    // TODO REFAC pointless message clones when not using result; filter.check
                    // should borrow and return bool/result
                    if let Some(_message) = filter.check(message.clone()) {
                        cdc_callback.1(payload);
                    }
                }

                for cdc_callback in self
                    .cdc_callbacks
                    .get_mut(&PostgresChangesEvent::All)
                    .unwrap_or(&mut vec![])
                {
                    let filter = &cdc_callback.0;

                    if let Some(_message) = filter.check(message.clone()) {
                        cdc_callback.1(payload);
                    }
                }
            }
            Payload::Broadcast(payload) => {
                if let Some(callbacks) = self.broadcast_callbacks.get_mut(&payload.event) {
                    for cb in callbacks {
                        cb(&payload.payload);
                    }
                }
            }
            _ => {}
        }

        match &message.event {
            MessageEvent::PhxClose => {
                if let Some(message_ref) = message.message_ref {
                    if message_ref == self.id.to_string() {
                        self.status = ChannelState::Closed;
                        if DEBUG {
                            println!("Channel Closed! {:?}", self.id);
                        }
                    }
                }
            }
            MessageEvent::PhxReply => {
                if message.message_ref.clone().unwrap_or("#NOREF".to_string())
                    == format!("{}+leave", self.id)
                {
                    self.status = ChannelState::Closed;
                    if DEBUG {
                        println!("Channel Closed! {:?}", self.id);
                    }
                }
            }
            _ => {}
        }
    }
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
    access_token: String,
    broadcast: BroadcastConfig,
    presence: PresenceConfig,
    id: Uuid,
    postgres_changes: Vec<PostgresChange>,
    cdc_callbacks: HashMap<PostgresChangesEvent, Vec<CdcCallback>>,
    broadcast_callbacks: HashMap<String, Vec<BroadcastCallback>>,
    presence_callbacks: HashMap<PresenceEvent, Vec<PresenceCallback>>,
    client_tx: futures_channel::mpsc::UnboundedSender<Message>,
}

impl RealtimeChannelBuilder {
    pub(crate) fn new(client: &mut RealtimeClient) -> Self {
        Self {
            topic: "no_topic".into(),
            access_token: client.access_token.clone(),
            broadcast: Default::default(),
            presence: Default::default(),
            id: Uuid::new_v4(),
            postgres_changes: Default::default(),
            cdc_callbacks: Default::default(),
            broadcast_callbacks: Default::default(),
            presence_callbacks: Default::default(),
            client_tx: client.get_channel_tx(),
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
        callback: impl FnMut(&PostgresChangesPayload) + 'static,
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
        callback: impl FnMut(String, PresenceState, PresenceState) + 'static,
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
        callback: impl FnMut(&HashMap<String, Value>) + 'static,
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
    pub async fn build(self, client: &mut RealtimeClient) -> Uuid {
        let (channel_tx, channel_rx) = futures_channel::mpsc::unbounded();

        client
            .add_channel(RealtimeChannel {
                topic: self.topic,
                cdc_callbacks: self.cdc_callbacks,
                broadcast_callbacks: self.broadcast_callbacks,
                client_tx: self.client_tx,
                channel_tx,
                channel_rx,
                status: ChannelState::Closed,
                id: self.id,
                join_payload: JoinPayload {
                    config: JoinConfig {
                        broadcast: self.broadcast,
                        presence: self.presence,
                        postgres_changes: self.postgres_changes,
                    },
                    access_token: self.access_token,
                },
                presence: RealtimePresence::from_channel_builder(self.presence_callbacks),
            })
            .await
    }
}
