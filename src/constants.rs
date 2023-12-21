// Copied this from realtime-js
// I'd assume we need the same enums and consts

use serde::{Deserialize, Serialize};

pub const DEFAULT_HEADERS: &str = "X-Client-Info: realtime-rs/0.1.0"; // TODO version

pub const VSN: &str = "1.0.0";

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

// TODO impl Display
/// Each should be evaluated as lowercase variant name
pub enum ChannelState {
    Closed,
    Errored,
    Joined,
    Joining,
    Leaving,
}

// TODO impl Display
#[derive(Serialize, Deserialize, Debug)]
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
}

// TODO impl Display = "websocket"
// TODO is this really needed tho
// i guess if gRPC is coming in future
pub enum Transport {
    WebSocket,
}
