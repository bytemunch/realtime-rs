use serde::{Deserialize, Serialize};
use tungstenite::Message;

use crate::message::payload::Payload;

#[derive(Serialize, Deserialize, Debug, Default, Clone)]
pub struct RealtimeMessage {
    pub event: MessageEvent,
    pub topic: String,
    // TODO payload structure
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
pub enum MessageEvent {
    #[serde(rename = "phx_close")]
    Close,
    #[serde(rename = "phx_error")]
    Error,
    #[serde(rename = "phx_join")]
    Join,
    #[serde(rename = "phx_reply")]
    Reply,
    #[serde(rename = "phx_leave")]
    Leave,
    #[serde(rename = "access_token")]
    AccessToken,
    #[serde(rename = "presence_state")]
    PresenceState,
    #[serde(rename = "system")]
    System,
    #[serde(rename = "heartbeat")]
    Heartbeat,
    #[serde(rename = "postgres_changes")]
    PostgresChanges,
    #[default]
    #[serde(rename = "broadcast")]
    Broadcast,
}
