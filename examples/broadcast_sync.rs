use std::{collections::HashMap, thread::sleep, time::Duration};

use realtime_rs::{
    message::payload::BroadcastConfig, realtime_channel::RealtimeChannelBuilder,
    realtime_client::RealtimeClientBuilder,
};

fn main() {
    env_logger::init();

    let endpoint = "http://127.0.0.1:54321";
    let access_token = std::env::var("LOCAL_ANON_KEY").unwrap();

    let client = RealtimeClientBuilder::new(endpoint, access_token)
        .set_heartbeat_interval(Duration::from_secs(29))
        .connect()
        .to_sync();

    let channel = RealtimeChannelBuilder::new("TestTopic")
        .set_broadcast_config(BroadcastConfig {
            broadcast_self: true,
            ack: false,
        })
        .on_broadcast("test_event", |map| println!("Event get! {:?}", map))
        .build_sync(&client)
        .unwrap();

    channel.subscribe();

    let mut payload = realtime_rs::message::payload::BroadcastPayload {
        event: "test_event".into(),
        payload: HashMap::new(),
        ..Default::default()
    };

    let mut count = 0;

    loop {
        count += 1;
        println!("SENDING {}", count);
        payload.payload.insert("count".into(), count.into());
        let _ = channel.broadcast(payload.clone());

        sleep(Duration::from_millis(1000));
    }
}
