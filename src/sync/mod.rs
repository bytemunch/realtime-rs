mod realtime_channel;
mod realtime_client;
mod realtime_presence;

pub use realtime_channel::*;
pub use realtime_client::*;
use tokio::sync::oneshot;

pub(crate) type Responder<T> = oneshot::Sender<T>;
