use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tungstenite::Message;

use crate::message::payload::Payload;

use super::payload::BroadcastPayload;

#[derive(Serialize, Deserialize, Debug, Default, Clone)]
pub struct RealtimeMessage {
    pub event: MessageEvent,
    pub topic: String,
    pub payload: Payload,
    #[serde(rename = "ref")]
    pub message_ref: Option<String>,
}

// TODO use derive-builder
impl RealtimeMessage {
    pub fn heartbeat() -> RealtimeMessage {
        RealtimeMessage {
            event: MessageEvent::Heartbeat,
            topic: "phoenix".to_owned(),
            payload: Payload::Empty {},
            message_ref: Some("".to_owned()),
        }
    }

    pub fn broadcast(event: String, payload: HashMap<String, Value>) -> RealtimeMessage {
        RealtimeMessage {
            event: MessageEvent::Broadcast,
            topic: "".into(),
            payload: Payload::Broadcast(BroadcastPayload::new(event, payload)),
            message_ref: None,
        }
    }

    pub fn new() -> Self {
        Self::default()
    }

    pub fn event(&mut self, event: MessageEvent) -> &mut Self {
        self.event = event;
        self
    }

    pub fn topic(&mut self, topic: String) -> &mut Self {
        self.topic = topic;
        self
    }

    pub fn payload(&mut self, payload: Payload) -> &mut Self {
        self.payload = payload;
        self
    }

    pub fn message_ref(&mut self, message_ref: Option<String>) -> &mut Self {
        self.message_ref = message_ref;
        self
    }
}

impl From<RealtimeMessage> for Message {
    fn from(val: RealtimeMessage) -> Self {
        let data = serde_json::to_string(&val).expect("Uhoh cannot into message");
        Message::Text(data)
    }
}

#[derive(Serialize, Deserialize, Debug, PartialEq, Default, Clone)]
#[serde(rename_all = "snake_case")]
pub enum MessageEvent {
    PhxClose,
    PhxError,
    PhxJoin,
    PhxReply,
    PhxLeave,
    AccessToken,
    Presence,
    System,
    Heartbeat,
    PostgresChanges,
    PresenceState,
    PresenceDiff,
    Track,
    Untrack,
    #[default]
    Broadcast,
}
