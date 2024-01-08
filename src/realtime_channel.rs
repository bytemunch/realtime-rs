use uuid::Uuid;

use crate::message::message_filter::{MessageFilter, MessageFilterEvent};
use crate::message::payload::{
    JoinConfig, JoinConfigBroadcast, JoinConfigPresence, JoinPayload, Payload, PayloadStatus,
    PostgresChange,
};
use crate::message::realtime_message::{MessageEvent, RealtimeMessage};
use crate::realtime_client::RealtimeClient;
use std::fmt::Debug;
use std::sync::mpsc::{self, SendError};

pub type RealtimeCallback = (
    MessageEvent,
    MessageFilter,
    Box<dyn FnMut(&RealtimeMessage)>,
);

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
    pub callbacks: Vec<RealtimeCallback>, // TODO 2d vec [event][callback] for better memory access
    tx: mpsc::Sender<RealtimeMessage>,
    postgres_changes: Vec<PostgresChange>,
    pub status: ChannelState,
    pub id: Uuid,
}

impl RealtimeChannel {
    pub(crate) fn new(
        client: &mut RealtimeClient,
        topic: String,
    ) -> Result<RealtimeChannel, ChannelCreateError> {
        let id = uuid::Uuid::new_v4();

        let Some(tx) = &client.outbound_tx else {
            println!("Client not ready.");
            return Err(ChannelCreateError::ClientNotReady);
        };

        Ok(RealtimeChannel {
            topic,
            callbacks: vec![],
            tx: tx.clone(),
            postgres_changes: vec![],
            status: ChannelState::Closed,
            id,
        })
    }

    pub fn subscribe(&mut self) -> Uuid {
        let join_message = RealtimeMessage {
            event: MessageEvent::Join,
            topic: self.topic.clone(),
            payload: Payload::Join(JoinPayload {
                config: JoinConfig {
                    presence: JoinConfigPresence {
                        key: "test_key".to_owned(),
                    },
                    broadcast: JoinConfigBroadcast {
                        broadcast_self: true,
                        ack: false,
                    },
                    postgres_changes: self.postgres_changes.clone(),
                },
            }),
            message_ref: Some(self.id.into()),
        };

        let _ = self.tx.send(join_message.into());

        self.id
    }

    pub fn unsubscribe(&mut self) -> Result<ChannelState, ChannelSendError> {
        if self.status == ChannelState::Closed || self.status == ChannelState::Leaving {
            return Ok(self.status);
        }

        match self.send(RealtimeMessage {
            event: MessageEvent::Leave,
            topic: self.topic.clone(),
            payload: Payload::Empty {},
            message_ref: Some(format!("{}+leave", self.id)),
        }) {
            Ok(()) => {
                self.status = ChannelState::Leaving;
                Ok(self.status)
            }
            Err(ChannelSendError::ChannelError(status)) => return Ok(status),
            // TODO too verbose
            Err(ChannelSendError::SendError(e)) => return Err(ChannelSendError::SendError(e)),
        }
    }

    pub fn presence_state(&self) {
        // TODO
    }

    pub fn track(&mut self) {
        // TODO
    }

    pub fn untrack(&mut self) {
        // TODO
    }

    pub fn send(&self, message: RealtimeMessage) -> Result<(), ChannelSendError> {
        if self.status == ChannelState::Closed || self.status == ChannelState::Leaving {
            return Err(ChannelSendError::ChannelError(self.status)); // TODO error here
        }

        match self.tx.send(message) {
            Ok(()) => Ok(()),
            Err(e) => Err(ChannelSendError::SendError(e)),
        }
    }

    pub fn on(
        &mut self,
        event: MessageEvent,
        filter: MessageFilter,
        callback: impl FnMut(&RealtimeMessage) + 'static,
    ) -> &mut Self {
        // modify self.postgres_changes with callback requirements

        if let MessageFilterEvent::PostgresCDC(cdc_event) = filter.event.clone() {
            self.postgres_changes.push(PostgresChange {
                event: cdc_event,
                schema: filter.schema.clone(),
                table: filter.table.clone().unwrap_or("".into()),
                // TODO filter filter filter, you know the filter filter, in the filter. dot clone.
                filter: filter.filter.clone(),
            });
        }

        self.callbacks.push((event, filter, Box::new(callback)));

        return self;
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
            _ => {}
        }

        match &message.event {
            MessageEvent::Close => {
                self.status = ChannelState::Closed;
                println!("Channel Closed! {:?}", self.id);
            }
            MessageEvent::Reply => {
                if &message.message_ref.clone().unwrap_or("#NOREF".to_string())
                    == &format!("{}+leave", self.id)
                {
                    self.status = ChannelState::Closed;
                    println!("Channel Closed! {:?}", self.id);
                }
            }
            _ => {
                for (event, filter, callback) in &mut self.callbacks {
                    // TODO clone clone clone clone clone
                    if let Some(message) = filter.clone().check(message.clone()) {
                        if *event == message.event {
                            callback(&message);
                        }
                    }
                }
            }
        }
    }
}

// Shut up compiler i dont know what im doing here okay
impl Debug for RealtimeChannel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&format!(
            "RealtimeChannel {{ name: {:?}, callbacks: [TODO DEBUG]}}",
            self.topic
        ))
    }
}
