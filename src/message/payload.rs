use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::sync::realtime_presence::{PresenceEvent, RawPresenceDiff, RawPresenceState};

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
    response: Value,
    status: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PresenceTrackPayload {
    event: PresenceEvent,
    payload: HashMap<String, Value>,
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

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct BroadcastPayload {
    pub event: String,
    pub payload: HashMap<String, Value>,
    #[serde(rename = "type")]
    pub broadcast_type: String, // TODO this is always 'broadcast', impl custom serde ;_;
}

impl BroadcastPayload {
    pub fn new(event: String, payload: HashMap<String, Value>) -> Self {
        BroadcastPayload {
            event,
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

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PostgresChangesPayload {
    pub data: PostgresChangeData,
    ids: Vec<usize>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PostgresChangeData {
    columns: Vec<PostgresColumn>,
    commit_timestamp: String,
    errors: Option<String>,
    old_record: Option<PostgresOldDataRef>,
    record: Option<HashMap<String, Value>>,
    #[serde(rename = "type")]
    pub change_type: PostgresChangesEvent,
    pub schema: String,
    pub table: String,
}

#[derive(Serialize, Deserialize, Debug)]
enum RecordValue {
    Bool(bool),
    Number(isize),
    String(String),
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct PostgresColumn {
    name: String,
    #[serde(rename = "type")]
    column_type: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct PostgresOldDataRef {
    id: isize,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct AccessTokenPayload {
    pub access_token: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SystemPayload {
    channel: String,
    extension: String,
    message: String,
    status: PayloadStatus,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct JoinPayload {
    pub config: JoinConfig,
    pub access_token: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct JoinConfig {
    pub broadcast: BroadcastConfig,
    pub presence: PresenceConfig,
    pub postgres_changes: Vec<PostgresChange>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct BroadcastConfig {
    #[serde(rename = "self")]
    pub(crate) broadcast_self: bool,
    pub(crate) ack: bool,
}

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
    pub response: JoinResponse,
    pub status: PayloadStatus,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct JoinResponse {
    postgres_changes: Vec<PostgresChange>,
}

#[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
pub enum PayloadStatus {
    #[serde(rename = "ok")]
    Ok,
    #[serde(rename = "error")]
    Error,
}
