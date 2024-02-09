use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::presence::{PresenceEvent, RawPresenceDiff, RawPresenceState};

/// Message payload, enum allows each payload type to be contained in
/// [crate::message::realtime_message::RealtimeMessage] without
/// needing a seperate struct per message type.
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(untagged)]
pub enum Payload {
    Join(JoinPayload),
    Response(JoinResponsePayload),
    System(SystemPayload),
    AccessToken(AccessTokenPayload),
    PostgresChanges(PostgresChangesPayload),
    Broadcast(BroadcastPayload),
    PresenceState(RawPresenceState),
    PresenceDiff(RawPresenceDiff),
    Reply(ReplyPayload),                 // think is only used for heartbeat?
    PresenceTrack(PresenceTrackPayload), // TODO matches greedily
    Empty {}, // TODO implement custom deser cos this bad. typechecking: this matches
              // everything that can't deser elsewhere. not good.
}

impl Default for Payload {
    fn default() -> Self {
        Payload::Broadcast(BroadcastPayload::default())
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ReplyPayload {
    pub response: Value,
    pub status: String,
}

/// Data to track with presence
///
/// Use [crate::sync::realtime_channel::RealtimeChannel::track()]
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PresenceTrackPayload {
    pub event: PresenceEvent, // TODO custom serialize to remove required event here
    pub payload: HashMap<String, Value>,
}

impl Default for PresenceTrackPayload {
    fn default() -> Self {
        Self {
            event: PresenceEvent::Track,
            payload: HashMap::new(),
        }
    }
}

impl From<HashMap<String, Value>> for PresenceTrackPayload {
    fn from(value: HashMap<String, Value>) -> Self {
        PresenceTrackPayload {
            payload: value,
            ..Default::default()
        }
    }
}

/// Payload for broadcast messages
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct BroadcastPayload {
    pub event: String,
    pub payload: HashMap<String, Value>,
    #[serde(rename = "type")]
    pub broadcast_type: String, // TODO this is always 'broadcast', impl custom serde ;_;
}

// TODO impl From<HashMap<String, Value>>

impl BroadcastPayload {
    pub fn new(event: impl Into<String>, payload: HashMap<String, Value>) -> Self {
        BroadcastPayload {
            event: event.into(),
            payload,
            broadcast_type: "broadcast".into(),
        }
    }
}

impl Default for BroadcastPayload {
    fn default() -> Self {
        BroadcastPayload {
            event: "event_missing".into(),
            payload: HashMap::new(),
            broadcast_type: "broadcast".into(),
        }
    }
}

/// Payload wrapper for postgres changes
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PostgresChangesPayload {
    pub data: PostgresChangeData,
    pub ids: Vec<usize>,
}

/// Recieved data regarding a postgres change
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PostgresChangeData {
    pub columns: Vec<PostgresColumn>,
    pub commit_timestamp: String,
    pub errors: Option<String>,
    pub old_record: Option<PostgresOldDataRef>,
    pub record: Option<HashMap<String, Value>>,
    #[serde(rename = "type")]
    pub change_type: PostgresChangesEvent,
    pub schema: String,
    pub table: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PostgresColumn {
    pub name: String,
    #[serde(rename = "type")]
    pub column_type: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PostgresOldDataRef {
    pub id: isize,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct AccessTokenPayload {
    pub access_token: String,
}

/// Subscription result payload
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SystemPayload {
    pub channel: String,
    pub extension: String,
    pub message: String,
    pub status: PayloadStatus,
}

/// Subscription configuration payload wrapper
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct JoinPayload {
    pub config: JoinConfig,
    pub access_token: String,
}

/// Subscription configuration data
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct JoinConfig {
    pub broadcast: BroadcastConfig,
    pub presence: PresenceConfig,
    pub postgres_changes: Vec<PostgresChange>,
}

/// Channel broadcast options
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct BroadcastConfig {
    #[serde(rename = "self")]
    pub broadcast_self: bool,
    pub ack: bool,
}

/// Channel presence options
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct PresenceConfig {
    pub key: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Eq, PartialEq, Default, Clone, Hash)]
pub enum PostgresChangesEvent {
    #[serde(rename = "*")]
    #[default]
    All,
    #[serde(rename = "INSERT")]
    Insert,
    #[serde(rename = "UPDATE")]
    Update,
    #[serde(rename = "DELETE")]
    Delete,
}

#[derive(Serialize, Deserialize, Debug, Default, Clone)]
pub struct PostgresChange {
    pub event: PostgresChangesEvent,
    pub schema: String,
    pub table: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filter: Option<String>, // TODO structured filters
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct JoinResponsePayload {
    pub response: PostgresChangesList,
    pub status: PayloadStatus,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PostgresChangesList {
    pub postgres_changes: Vec<PostgresChange>,
}

#[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
pub enum PayloadStatus {
    #[serde(rename = "ok")]
    Ok,
    #[serde(rename = "error")]
    Error,
}
