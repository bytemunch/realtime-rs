use crate::realtime_client::Payload::PostgresChange;
use crate::realtime_client::{PostgresEvent, RealtimeMessage};
use std::fmt::Debug;

pub struct RealtimeChannel {
    pub topic: String,
    pub callbacks: Vec<(PostgresEvent, Box<dyn FnMut(&RealtimeMessage)>)>, // TODO this is not the right memory
                                                                           // layout, or something,
                                                                           // probably
}

impl RealtimeChannel {
    pub fn new(topic: String) -> RealtimeChannel {
        RealtimeChannel {
            topic,
            callbacks: vec![],
        }
    }

    pub fn on(&mut self, event: PostgresEvent, callback: impl FnMut(&RealtimeMessage) + 'static) {
        self.callbacks.push((event, Box::new(callback)));
    }

    pub fn recieve(&mut self, message: RealtimeMessage) {
        let PostgresChange(ref payload) = message.payload else {
            return;
        };

        for cb in &mut self.callbacks {
            if cb.0 == payload.data.change_type {
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
