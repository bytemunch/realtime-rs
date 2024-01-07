use serde::{Deserialize, Serialize};

use crate::message::{
    payload::{Payload, PostgresEvent},
    realtime_message::{MessageEvent, RealtimeMessage},
};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum MessageFilterEvent {
    PostgresCDC(PostgresEvent),
    MessageEvent(MessageEvent),
    Custom(String),
}

impl Default for MessageFilterEvent {
    fn default() -> Self {
        MessageFilterEvent::Custom("".into())
    }
}

// TODO builder pattern for filter?
#[derive(Serialize, Deserialize, Debug, Default, Clone)]
pub struct MessageFilter {
    pub event: MessageFilterEvent,
    pub schema: String,
    pub table: Option<String>,
    pub filter: Option<String>,
}

impl MessageFilter {
    pub fn check(self, message: RealtimeMessage) -> Option<RealtimeMessage> {
        match self.event {
            MessageFilterEvent::PostgresCDC(postgres_event) => {
                let Payload::PostgresChange(payload) = &message.payload else {
                    println!("Dropping non CDC message: {:?}", message);
                    return None;
                };

                if let Some(table) = self.table {
                    if table != payload.data.table {
                        println!("Dropping mismatched table message: {:?}", message);
                        return None;
                    }
                }

                if let Some(_filter) = self.filter {
                    // Filters do not need to be checked client-side
                }

                if (postgres_event == PostgresEvent::All
                    || payload.data.change_type == postgres_event)
                    && payload.data.schema == self.schema
                {
                    return Some(message);
                }

                println!("Dropping mismatched CDC event: {:?}", message);
            }
            MessageFilterEvent::MessageEvent(event) => {
                println!("\nGot event: {:?}\n", event);
            }
            MessageFilterEvent::Custom(event) => {
                let msg_event_string = serde_json::to_string(&message.event).expect("whoops");

                if let Payload::Broadcast(payload) = message.payload.clone() {
                    // match internal tings
                    if event == payload.event {
                        return Some(message);
                    }
                }

                if event == msg_event_string {
                    return Some(message);
                }

                println!(
                    "Dropping mismatched non-CDC event: {:?}\n{:?} | {:?}",
                    message, event, msg_event_string
                );
            }
        }

        None
    }
}
