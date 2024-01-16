use serde::{Deserialize, Serialize};

use crate::message::{payload::Payload, realtime_message::RealtimeMessage};

// TODO builder pattern for filter?
#[derive(Serialize, Deserialize, Debug, Default, Clone)]
pub struct CdcMessageFilter {
    pub schema: String,
    pub table: Option<String>,
    pub filter: Option<String>,
}

impl CdcMessageFilter {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn schema(mut self, schema: String) -> Self {
        self.schema = schema;
        self
    }
    pub fn table(mut self, table: String) -> Self {
        self.table = Some(table);
        self
    }
    pub fn filter(mut self, filter: String) -> Self {
        self.filter = Some(filter);
        self
    }
}

impl CdcMessageFilter {
    pub fn check(&self, message: RealtimeMessage) -> Option<RealtimeMessage> {
        let Payload::PostgresChanges(payload) = &message.payload else {
            println!("Dropping non CDC message: {:?}", message);
            return None;
        };

        if let Some(table) = &self.table {
            if table != &payload.data.table {
                println!("Dropping mismatched table message: {:?}", message);
                return None;
            }
        }

        if let Some(_filter) = &self.filter {
            // Filters do not need to be checked client-side
        }

        if payload.data.schema != self.schema {
            return None;
        }

        println!("Dropping mismatched CDC event: {:?}", message);

        Some(message)
    }
}
