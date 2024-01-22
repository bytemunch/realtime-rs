use serde::{Deserialize, Serialize};
use tungstenite::Message;

use crate::message::payload::Payload;

/// Structure of messages sent to and from the server
#[derive(Serialize, Deserialize, Debug, Default, Clone)]
pub struct RealtimeMessage {
    pub event: MessageEvent,
    pub topic: String,
    pub payload: Payload,
    #[serde(rename = "ref")]
    pub message_ref: Option<String>,
}

impl RealtimeMessage {
    pub(crate) fn heartbeat() -> RealtimeMessage {
        RealtimeMessage {
            event: MessageEvent::Heartbeat,
            topic: "phoenix".to_owned(),
            payload: Payload::Empty {},
            message_ref: Some("".to_owned()),
        }
    }
}

impl From<RealtimeMessage> for Message {
    fn from(val: RealtimeMessage) -> Self {
        let data = serde_json::to_string(&val).expect("Uhoh cannot into message");
        Message::Text(data)
    }
}

/// Realtime message event list
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
