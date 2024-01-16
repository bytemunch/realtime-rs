use serde_json::Value;
use uuid::Uuid;

use crate::message::cdc_message_filter::CdcMessageFilter;
use crate::message::payload::{
    AccessTokenPayload, JoinConfig, JoinConfigBroadcast, JoinConfigPresence, JoinPayload, Payload,
    PayloadStatus, PostgresChange, PostgresChangesEvent, PostgresChangesPayload,
};
use crate::message::realtime_message::{MessageEvent, RealtimeMessage};
use crate::sync::{realtime_client::RealtimeClient, realtime_presence::RealtimePresence};
use std::collections::HashMap;
use std::fmt::Debug;
use std::sync::mpsc::{self, SendError};

use super::realtime_presence::{PresenceEvent, PresenceState};

pub type CdcCallback = (CdcMessageFilter, Box<dyn FnMut(&PostgresChangesPayload)>);

#[derive(PartialEq, Clone, Copy, Debug)]
pub enum ChannelState {
    Closed,
    Errored,
    Joined,
    Joining,
    Leaving,
}

#[derive(Debug)]
pub enum ChannelSendError {
    SendError(SendError<RealtimeMessage>),
    ChannelError(ChannelState),
}

#[derive(Debug)]
pub enum ChannelCreateError {
    ClientNotReady,
}

pub struct RealtimeChannel {
    pub topic: String,
    cdc_callbacks: HashMap<PostgresChangesEvent, Vec<CdcCallback>>,
    broadcast_callbacks: HashMap<String, Vec<Box<dyn FnMut(&HashMap<String, Value>)>>>,
    tx: mpsc::Sender<RealtimeMessage>,
    pub status: ChannelState,
    pub id: Uuid,
    join_payload: JoinPayload,
    presence: RealtimePresence,
    message_queue: Vec<RealtimeMessage>,
}

// TODO channel options with broadcast + presence settings

impl RealtimeChannel {
    pub(crate) fn new(
        client: &mut RealtimeClient,
        topic: String,
    ) -> Result<RealtimeChannel, ChannelCreateError> {
        let id = uuid::Uuid::new_v4();

        let access_token = client.access_token.clone();

        Ok(RealtimeChannel {
            topic,
            cdc_callbacks: Default::default(),
            broadcast_callbacks: Default::default(),
            tx: client.outbound_channel.0.clone(),
            status: ChannelState::Closed,
            join_payload: JoinPayload {
                access_token: Some(access_token),
                config: JoinConfig {
                    presence: JoinConfigPresence {
                        key: Some("".into()),
                    },
                    broadcast: JoinConfigBroadcast {
                        broadcast_self: true,
                        ack: true,
                    },
                    ..Default::default()
                },
                ..Default::default()
            },
            id,
            presence: RealtimePresence::default(),
            message_queue: vec![],
        })
    }

    pub fn subscribe(&mut self) -> Uuid {
        let join_message = RealtimeMessage {
            event: MessageEvent::PhxJoin,
            topic: self.topic.clone(),
            payload: Payload::Join(self.join_payload.clone()),
            message_ref: Some(self.id.into()),
        };

        self.status = ChannelState::Joining;

        let _ = self.tx.send(join_message.into());

        self.id
    }

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
            Err(ChannelSendError::ChannelError(status)) => return Ok(status),
            Err(e) => return Err(e),
        }
    }

    pub fn set_auth(&mut self, access_token: String) -> Result<(), ChannelSendError> {
        self.join_payload.access_token = Some(access_token.clone());

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

    pub fn presence_state(&self) -> PresenceState {
        self.presence.state.clone().into()
    }

    pub fn track(&mut self, payload: HashMap<String, Value>) -> &mut RealtimeChannel {
        let _ = self.send(RealtimeMessage {
            event: MessageEvent::Presence,
            topic: self.topic.clone(),
            payload: Payload::PresenceTrack(payload.into()),
            message_ref: None,
        });

        self
    }

    pub fn untrack(&mut self) {
        let _ = self.send(RealtimeMessage {
            event: MessageEvent::Untrack,
            topic: self.topic.clone(),
            payload: Payload::Empty {},
            message_ref: None,
        });
    }

    pub fn send(&mut self, message: RealtimeMessage) -> Result<(), ChannelSendError> {
        // inject channel topic to message here
        let mut message = message.clone();
        message.topic = self.topic.clone();

        if self.status == ChannelState::Leaving {
            return Err(ChannelSendError::ChannelError(self.status));
        }
        // queue message
        // TODO use mpsc channel as queue, drain only when realtime channel ready?
        // prevents need for mut access when sending
        self.message_queue.push(message);

        self.drain_queue()
    }

    pub(crate) fn drain_queue(&mut self) -> Result<(), ChannelSendError> {
        if self.status != ChannelState::Joined && self.status != ChannelState::Joining {
            return Err(ChannelSendError::ChannelError(self.status));
        }

        while !self.message_queue.is_empty() {
            if let Err(e) = self.tx.send(self.message_queue.pop().unwrap()) {
                return Err(ChannelSendError::SendError(e));
            }
        }

        Ok(())
    }

    // TODO on_message handler for sys messages and presence_state and presence_diff

    pub fn on_broadcast(
        &mut self,
        event: String,
        callback: impl FnMut(&HashMap<String, Value>) + 'static,
    ) -> &mut RealtimeChannel {
        if let None = self.broadcast_callbacks.get_mut(&event) {
            self.broadcast_callbacks.insert(event.clone(), vec![]);
        }

        self.broadcast_callbacks
            .get_mut(&event)
            .unwrap_or(&mut vec![])
            .push(Box::new(callback));

        self
    }

    pub fn on_cdc(
        &mut self,
        event: PostgresChangesEvent,
        filter: CdcMessageFilter,
        callback: impl FnMut(&PostgresChangesPayload) + 'static,
    ) -> &mut Self {
        self.join_payload
            .config
            .postgres_changes
            .push(PostgresChange {
                event: event.clone(),
                schema: filter.schema.clone(),
                table: filter.table.clone().unwrap_or("".into()),
                filter: filter.filter.clone(),
            });

        if let None = self.cdc_callbacks.get_mut(&event) {
            self.cdc_callbacks.insert(event.clone(), vec![]);
        }

        self.cdc_callbacks
            .get_mut(&event)
            .unwrap_or(&mut vec![])
            .push((filter, Box::new(callback)));

        return self;
    }

    pub fn on_presence(
        &mut self,
        event: PresenceEvent,
        // TODO callback type alias
        callback: impl FnMut(String, PresenceState, PresenceState) + 'static,
    ) -> &mut RealtimeChannel {
        self.presence.add_callback(event, Box::new(callback));
        self
    }

    pub fn recieve(&mut self, message: RealtimeMessage) {
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

                for cdc_callback in self.cdc_callbacks.get_mut(&event).unwrap_or(&mut vec![]) {
                    let ref filter = cdc_callback.0;

                    // TODO REFAC pointless message clones when not using result; filter.check
                    // should borrow and return bool/result
                    if let Some(_message) = filter.check(message.clone()) {
                        cdc_callback.1(&payload);
                    }
                }

                for cdc_callback in self
                    .cdc_callbacks
                    .get_mut(&PostgresChangesEvent::All)
                    .unwrap_or(&mut vec![])
                {
                    let ref filter = cdc_callback.0;

                    if let Some(_message) = filter.check(message.clone()) {
                        cdc_callback.1(&payload);
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
                self.status = ChannelState::Closed;
                println!("Channel Closed! {:?}", self.id);
            }
            MessageEvent::PhxReply => {
                if &message.message_ref.clone().unwrap_or("#NOREF".to_string())
                    == &format!("{}+leave", self.id)
                {
                    self.status = ChannelState::Closed;
                    println!("Channel Closed! {:?}", self.id);
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
