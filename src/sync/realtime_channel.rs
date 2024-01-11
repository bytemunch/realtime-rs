use uuid::Uuid;

use crate::message::message_filter::{MessageFilter, MessageFilterEvent};
use crate::message::payload::{
    AccessTokenPayload, JoinPayload, Payload, PayloadStatus, PostgresChange,
};
use crate::message::realtime_message::{MessageEvent, RealtimeMessage};
use crate::sync::realtime_client::RealtimeClient;
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
    pub status: ChannelState,
    pub id: Uuid,
    join_payload: JoinPayload,
}

impl RealtimeChannel {
    pub(crate) fn new(
        client: &mut RealtimeClient,
        topic: String,
    ) -> Result<RealtimeChannel, ChannelCreateError> {
        let id = uuid::Uuid::new_v4();

        let access_token = client.access_token.clone();

        Ok(RealtimeChannel {
            topic,
            callbacks: vec![],
            tx: client.outbound_channel.0.clone(),
            status: ChannelState::Closed,
            join_payload: JoinPayload {
                access_token: Some(access_token),
                ..Default::default()
            },
            id,
        })
    }

    pub fn subscribe(&mut self) -> Uuid {
        let join_message = RealtimeMessage {
            event: MessageEvent::Join,
            topic: self.topic.clone(),
            payload: Payload::Join(self.join_payload.clone()),
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
            return Err(ChannelSendError::ChannelError(self.status));
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
        if let MessageFilterEvent::PostgresCDC(cdc_event) = filter.event.clone() {
            self.join_payload
                .config
                .postgres_changes
                .push(PostgresChange {
                    event: cdc_event,
                    schema: filter.schema.clone(),
                    table: filter.table.clone().unwrap_or("".into()),
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

impl Debug for RealtimeChannel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&format!(
            "RealtimeChannel {{ name: {:?}, callbacks: [TODO DEBUG]}}",
            self.topic
        ))
    }
}
