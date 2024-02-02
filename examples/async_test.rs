use std::{collections::HashMap, time::Duration};

use realtime_rs::{message::payload::BroadcastConfig, sync::RealtimeClient};

#[tokio::main]
async fn main() {
    let endpoint = "http://127.0.0.1:54321";
    let access_token = std::env::var("LOCAL_ANON_KEY").unwrap();

    let mut client = RealtimeClient::builder(endpoint, access_token)
        .heartbeat_interval(Duration::from_secs(1))
        .build();

    let _ = client.connect().await;

    let mut c = client
        .channel("TestTopic")
        .broadcast(BroadcastConfig {
            broadcast_self: true,
            ack: false,
        })
        .on_broadcast("test_event", |map| println!("Event get! {:?}", map))
        .build(&mut client)
        .await;

    c.subscribe(&mut client).await;

    let payload = realtime_rs::message::payload::BroadcastPayload {
        event: "test_event".into(),
        payload: HashMap::new(),
        ..Default::default()
    };

    let _ = c.broadcast(payload).await;

    let _ = client.handle_incoming().await;

    println!("client closed.");
}
