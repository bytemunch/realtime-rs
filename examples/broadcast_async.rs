use std::{collections::HashMap, time::Duration};

use realtime_rs::{
    message::payload::BroadcastConfig, realtime_channel::RealtimeChannelBuilder,
    realtime_client::RealtimeClientBuilder,
};
use tokio::time::sleep;

#[tokio::main]
async fn main() {
    let endpoint = "http://127.0.0.1:54321";
    let access_token = std::env::var("LOCAL_ANON_KEY").unwrap();

    let client = RealtimeClientBuilder::new(endpoint, access_token)
        .heartbeat_interval(Duration::from_secs(29))
        .build();

    client.connect().await;

    let channel = RealtimeChannelBuilder::new("TestTopic")
        .broadcast(BroadcastConfig {
            broadcast_self: true,
            ack: false,
        })
        .on_broadcast("test_event", |map| println!("Event get! {:?}", map))
        .build(&client)
        .await
        .unwrap();

    channel.subscribe_blocking().await.unwrap();

    let mut payload = realtime_rs::message::payload::BroadcastPayload {
        event: "test_event".into(),
        payload: HashMap::new(),
        ..Default::default()
    };

    let mut count = 0;

    tokio::spawn(async move {
        loop {
            count += 1;
            println!("SENDING {}", count);
            payload.payload.insert("count".into(), count.into());
            let _ = channel.broadcast(payload.clone());
            sleep(Duration::from_millis(1000)).await;
        }
    })
    .await
    .unwrap();
}