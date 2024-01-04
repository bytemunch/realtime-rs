use crate::constants::MessageEvent;
use crate::realtime_client::Payload::{self};
use crate::realtime_client::{
    JoinConfig, JoinConfigBroadcast, JoinConfigPresence, JoinPayload, PostgresChange,
    PostgresEvent, RealtimeClient, RealtimeMessage,
};
use std::fmt::Debug;

pub struct RealtimeChannel {
    pub topic: String,
    pub callbacks: Vec<(PostgresEvent, Box<dyn FnMut(&RealtimeMessage)>)>, // TODO this is not the right memory
                                                                           // layout, or something,
                                                                           // probably
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
            callbacks: vec![],
        }
    }

    pub fn on(&mut self, event: PostgresEvent, callback: impl FnMut(&RealtimeMessage) + 'static) {
        println!("Registered {:?} callback.", event);
        self.callbacks.push((event, Box::new(callback)));
    }

    pub fn recieve(&mut self, message: RealtimeMessage) {
        let Payload::PostgresChange(ref payload) = message.payload else {
            println!("Channel dropping message: {:?}", message);
            return;
        };

        for cb in &mut self.callbacks {
            if cb.0 == payload.data.change_type || cb.0 == PostgresEvent::All {
                cb.1(&message);
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
