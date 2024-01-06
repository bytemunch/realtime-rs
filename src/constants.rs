// Copied this from realtime-js
// I'd assume we need the same enums and consts

use serde::{Deserialize, Serialize};

pub const DEFAULT_TIMEOUT: usize = 10000; // TODO duration type?

pub const WS_CLOSE_NORMAL: usize = 1000;

// TODO impl Display as lowercase variant names
pub enum SocketState {
    Connecting,
    Open,
    Closing,
    Closed,
}

// ^^^ socket state / connection state (same thing)
pub enum ConnectionState {
    Connecting,
    Open,
    Closing,
    Closed,
}

#[derive(PartialEq)]
pub enum ChannelState {
    Closed,
    Errored,
    Joined,
    Joining,
    Leaving,
}

// TODO impl Display
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

// TODO impl Display = "websocket"
// TODO is this really needed tho
// i guess if gRPC is coming in future
pub enum Transport {
    WebSocket,
}
