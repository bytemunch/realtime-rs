use serde::{Deserialize, Serialize};
use tungstenite::Message;

use crate::message::payload::Payload;

#[derive(Serialize, Deserialize, Debug, Default, Clone)]
pub struct RealtimeMessage {
    pub event: MessageEvent,
    pub topic: String,
    pub payload: Payload,
    #[serde(rename = "ref")]
    pub message_ref: Option<String>,
}

impl RealtimeMessage {
    pub fn heartbeat() -> RealtimeMessage {
        RealtimeMessage {
            event: MessageEvent::Heartbeat,
            topic: "phoenix".to_owned(),
            payload: Payload::Empty {},
            message_ref: Some("".to_owned()),
        }
    }
}

impl Into<Message> for RealtimeMessage {
    fn into(self) -> Message {
        let data = serde_json::to_string(&self).expect("Uhoh cannot into message");
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
