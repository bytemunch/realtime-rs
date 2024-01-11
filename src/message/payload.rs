use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(untagged)]
pub enum Payload {
    Join(JoinPayload),
    Response(JoinResponsePayload),
    System(SystemPayload),
    AccessToken(AccessTokenPayload),
    PostgresChange(PostgresChangePayload), // TODO rename because clashes
    Broadcast(BroadcastPayload),
    Empty {}, // TODO perf: implement custom deser cos this bad. typechecking: this matches
              // everything that can't deser elsewhere. not good.
}

impl Default for Payload {
    fn default() -> Self {
        Payload::Broadcast(BroadcastPayload::default())
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct BroadcastPayload {
    pub event: String,
    pub payload: HashMap<String, Value>,
    #[serde(rename = "type")]
    pub broadcast_type: String, // TODO this is always 'broadcast', impl custom serde ;_;
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
pub struct PostgresChangePayload {
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
    pub change_type: PostgresEvent,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub access_token: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct JoinConfig {
    pub broadcast: JoinConfigBroadcast,
    pub presence: JoinConfigPresence,
    pub postgres_changes: Vec<PostgresChange>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct JoinConfigBroadcast {
    #[serde(rename = "self")]
    pub(crate) broadcast_self: bool,
    pub(crate) ack: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct JoinConfigPresence {
    pub key: String,
}

#[derive(Serialize, Deserialize, Debug, Eq, PartialEq, Default, Clone)]
pub enum PostgresEvent {
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
    pub event: PostgresEvent,
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
