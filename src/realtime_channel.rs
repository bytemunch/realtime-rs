use uuid::Uuid;

use crate::constants::MessageEvent;
use crate::realtime_client::Payload::{self};
use crate::realtime_client::{
    JoinConfig, JoinConfigBroadcast, JoinConfigPresence, JoinPayload, PostgresChange,
    PostgresEvent, RealtimeClient, RealtimeMessage,
};
use std::collections::HashMap;
use std::fmt::Debug;

pub type RealtimeCallback = (PostgresEvent, Box<dyn FnMut(&RealtimeMessage)>);

pub struct RealtimeChannel {
    pub topic: String,
    pub callbacks: HashMap<Uuid, RealtimeCallback>,
}

impl RealtimeChannel {
    pub fn new(
        client: &mut RealtimeClient,
        topic: String,
        postgres_changes: Vec<PostgresChange>,
    ) -> RealtimeChannel {
        let join_message = RealtimeMessage {
            event: MessageEvent::Join,
            topic: topic.clone(),
            payload: Payload::Join(JoinPayload {
                config: JoinConfig {
                    presence: JoinConfigPresence {
                        key: "test_key".to_owned(),
                    },
                    broadcast: JoinConfigBroadcast {
                        broadcast_self: true,
                        ack: false,
                    },
                    postgres_changes,
                },
            }),
            message_ref: Some("init".to_owned()), // TODO idk what this does
        };

        client.send(join_message.into());

        RealtimeChannel {
            topic,
            callbacks: HashMap::new(),
        }
    }

    pub fn on(
        &mut self,
        event: PostgresEvent,
        callback: impl FnMut(&RealtimeMessage) + 'static,
    ) -> Uuid {
        let id = Uuid::new_v4();
        self.callbacks.insert(id, (event, Box::new(callback)));
        println!("Registered callback {:?}", id);
        return id;
    }

    pub fn drop(&mut self, uuid: Uuid) -> Result<RealtimeCallback, &str> {
        println!("Removed callback {:?}", uuid);
        match self.callbacks.remove(&uuid) {
            Some(callback) => Ok(callback),
            None => Err("Callback not found."),
        }
    }

    pub fn recieve(&mut self, message: RealtimeMessage) {
        let Payload::PostgresChange(ref payload) = message.payload else {
            println!("Channel dropping message: {:?}", message);
            return;
        };

        for cb in &mut self.callbacks {
            // TODO chained numerical accessors ewwwwwwwwwww
            if cb.1 .0 == payload.data.change_type || cb.1 .0 == PostgresEvent::All {
                cb.1 .1(&message);
            }
        }
    }
}

// Shut up compiler i dont know what im doing here okay
impl Debug for RealtimeChannel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&format!(
            "RealtimeChannel {{ name: {:?}, callbacks: [TODO DEBUG]}}",
            self.topic
        ))
    }
}
