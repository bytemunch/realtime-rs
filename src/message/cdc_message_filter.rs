use serde::{Deserialize, Serialize};

use crate::{
    message::{payload::Payload, realtime_message::RealtimeMessage},
    DEBUG,
};

/// Incoming message filter for local callbacks
///```
/// # use realtime_rs::{
/// #     message::{cdc_message_filter::CdcMessageFilter, payload::PostgresChangesEvent},
/// #     sync::realtime_client::{ConnectionState, NextMessageError, RealtimeClient},
/// # };
/// # use std::env;
/// #
/// # fn main() -> Result<(), ()> {
/// #     let url = "http://127.0.0.1:54321".into();
/// #     let anon_key = env::var("LOCAL_ANON_KEY").expect("No anon key!");
/// #     let mut client = RealtimeClient::builder(url, anon_key).build();
/// #     let _ = client.connect();
///     let my_cdc_callback = move |msg: &_| {
///         println!("Got message: {:?}", msg);
///     };
///
///     let channel_id = client
///         .channel("topic".into())
///         .on_cdc(
///             PostgresChangesEvent::All,
///             CdcMessageFilter {
///                 schema: "public".into(),
///                 table: Some("todos".into()),
///                 ..Default::default()
///             },
///             my_cdc_callback,
///         )
///         .build(&mut client);
/// #
/// #     client.get_channel_mut(channel_id).unwrap().subscribe();
/// #     loop {
/// #         if client.get_status() == ConnectionState::Closed {
/// #             break;
/// #         }
/// #         match client.next_message() {
/// #             Ok(_topic) => return Ok(()),
/// #             Err(NextMessageError::WouldBlock) => return Ok(()),
/// #             Err(_e) => return Err(()),
/// #         }
/// #     }
/// #     Err(())
/// # }
#[derive(Serialize, Deserialize, Debug, Default, Clone)]
pub struct CdcMessageFilter {
    pub schema: String,
    pub table: Option<String>,
    pub filter: Option<String>,
}

impl CdcMessageFilter {
    pub(crate) fn check(&self, message: RealtimeMessage) -> Option<RealtimeMessage> {
        let Payload::PostgresChanges(payload) = &message.payload else {
            if DEBUG {
                println!("Dropping non CDC message: {:?}", message);
            }
            return None;
        };

        if let Some(table) = &self.table {
            if table != &payload.data.table {
                if DEBUG {
                    println!("Dropping mismatched table message: {:?}", message);
                }
                return None;
            }
        }

        if let Some(_filter) = &self.filter {
            // Filters do not need to be checked client-side
        }

        if payload.data.schema != self.schema {
            return None;
        }

        if DEBUG {
            println!("Dropping mismatched CDC event: {:?}", message);
        }

        Some(message)
    }
}
