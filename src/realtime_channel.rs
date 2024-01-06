use uuid::Uuid;

use crate::constants::{ChannelState, MessageEvent};
use crate::realtime_client::Payload::{self};
use crate::realtime_client::{
    JoinConfig, JoinConfigBroadcast, JoinConfigPresence, JoinPayload, MessageFilter,
    MessageFilterEvent, PayloadStatus, PostgresChange, RealtimeClient, RealtimeMessage,
};
use std::collections::HashMap;
use std::fmt::Debug;
use std::sync::mpsc;

pub type RealtimeCallback = (
    MessageEvent,
    MessageFilter,
    Box<dyn FnMut(&RealtimeMessage)>,
);

pub struct RealtimeChannel {
    pub topic: String,
    pub callbacks: HashMap<Uuid, RealtimeCallback>,
    tx: mpsc::Sender<RealtimeMessage>,
    postgres_changes: Vec<PostgresChange>,
    pub status: ChannelState,
    pub id: Uuid,
}

impl RealtimeChannel {
    pub(crate) fn new(client: &mut RealtimeClient, topic: String) -> RealtimeChannel {
        let id = uuid::Uuid::new_v4();

        RealtimeChannel {
            topic,
            callbacks: HashMap::new(),
            tx: client.outbound_tx.clone(),
            postgres_changes: vec![],
            status: ChannelState::Closed,
            id,
        }
    }

    pub fn subscribe(&mut self) -> Uuid {
        let join_message = RealtimeMessage {
            event: MessageEvent::Join,
            topic: self.topic.clone(),
            payload: Payload::Join(JoinPayload {
                config: JoinConfig {
                    presence: JoinConfigPresence {
                        key: "test_key".to_owned(),
                    },
                    broadcast: JoinConfigBroadcast {
                        broadcast_self: true,
                        ack: false,
                    },
                    postgres_changes: self.postgres_changes.clone(),
                },
            }),
            message_ref: Some(self.id.into()),
        };

        let _ = self.tx.send(join_message.into());

        self.id
    }

    pub fn send(&self, message: RealtimeMessage) -> Result<(), mpsc::SendError<RealtimeMessage>> {
        self.tx.send(message)
    }

    pub fn on(
        &mut self,
        event: MessageEvent,
        filter: MessageFilter,
        callback: impl FnMut(&RealtimeMessage) + 'static,
    ) -> &mut Self {
        // TODO don't need to ref callbacks so no ID needed, restructure callback map
        let id = Uuid::new_v4();

        // modify self.postgres_changes with callback requirements

        if let MessageFilterEvent::PostgresCDC(cdc_event) = filter.event.clone() {
            self.postgres_changes.push(PostgresChange {
                event: cdc_event,
                schema: filter.schema.clone(),
                table: filter.table.clone().unwrap_or("".into()),
                // TODO filter filter filter, you know the filter filter, in the filter. dot clone.
                filter: filter.filter.clone(),
            });
        }

        self.callbacks
            .insert(id, (event, filter, Box::new(callback)));

        println!("Registered callback {:?}", id);
        return self;
    }

    pub fn recieve(&mut self, message: RealtimeMessage) {
        match &message.payload {
            Payload::Response(join_response) => {
                let target_id = message.message_ref.clone().unwrap_or("".to_string());
                if target_id != self.id.to_string() {
                    return;
                }
                if join_response.status == PayloadStatus::Ok {
                    self.status = ChannelState::Joined;
                }
            }
            _ => {}
        }

        // TODO redo this callback system to incorporate new API and filters
        for (_key, (event, _filter, callback)) in &mut self.callbacks {
            if *event == message.event {
                callback(&message);
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
