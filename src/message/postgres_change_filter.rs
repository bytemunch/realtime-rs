use serde::{Deserialize, Serialize};

use crate::{
    message::{payload::Payload, realtime_message::RealtimeMessage},
    DEBUG,
};

/// Incoming message filter for local callbacks
#[derive(Serialize, Deserialize, Debug, Default, Clone)]
pub struct PostgresChangeFilter {
    pub schema: String,
    pub table: Option<String>,
    pub filter: Option<String>,
}

impl PostgresChangeFilter {
    pub(crate) fn check(&self, message: &RealtimeMessage) -> bool {
        let Payload::PostgresChanges(payload) = &message.payload else {
            if DEBUG {
                println!("Dropping non CDC message: {:?}", message);
            }
            return false;
        };

        if let Some(table) = &self.table {
            if table != &payload.data.table {
                if DEBUG {
                    println!("Dropping mismatched table message: {:?}", message);
                }
                return false;
            }
        }

        if let Some(_filter) = &self.filter {
            // Filters do not need to be checked client-side
        }

        if payload.data.schema != self.schema {
            return false;
        }

        if DEBUG {
            println!("Dropping mismatched CDC event: {:?}", message);
        }

        true
    }
}
