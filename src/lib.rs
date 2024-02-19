use tokio::sync::oneshot;

pub(crate) type Responder<T> = oneshot::Sender<T>;

pub mod message;
pub mod realtime_channel;
pub mod realtime_client;
pub mod realtime_presence;
