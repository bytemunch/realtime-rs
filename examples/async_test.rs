use std::{collections::HashMap, time::Duration};

use realtime_rs::{
    message::payload::BroadcastConfig,
    sync::{ChannelControlMessage, RealtimeClient},
};
use tokio::time::sleep;

#[tokio::main]
async fn main() {
    let endpoint = "http://127.0.0.1:54321";
    let access_token = std::env::var("LOCAL_ANON_KEY").unwrap();

    let mut client = RealtimeClient::builder(endpoint, access_token)
        .heartbeat_interval(Duration::from_secs(1))
        .build();

    let _ = client.connect().await;

    let c = client
        .channel("TestTopic")
        .broadcast(BroadcastConfig {
            broadcast_self: true,
            ack: false,
        })
        .on_broadcast("test_event", |map| println!("Event get! {:?}", map))
        .build(&mut client)
        .await;

    let _ = c.subscribe_blocking();

    let mut payload = realtime_rs::message::payload::BroadcastPayload {
        event: "test_event".into(),
        payload: HashMap::new(),
        ..Default::default()
    };

    let mut count = 0;

    tokio::spawn(async move {
        loop {
            sleep(Duration::from_secs(2)).await;
            count += 1;
            println!("SENDING {}", count);
            payload.payload.insert("count".into(), count.into());
            let _ = c.send(ChannelControlMessage::Broadcast(payload.clone()));
        }
    });

    let _ = client.handle_incoming().await;

    println!("client closed.");
}
