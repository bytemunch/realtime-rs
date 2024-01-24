use std::{collections::HashMap, env};

use realtime_rs::{
    message::payload::{BroadcastConfig, BroadcastPayload},
    sync::{NextMessageError, RealtimeClient},
};

fn main() {
    let url = "http://127.0.0.1:54321";
    let anon_key = env::var("LOCAL_ANON_KEY").expect("No anon key!");

    let mut client = RealtimeClient::builder(url, anon_key).build();

    let _ = client.connect();

    let channel_a = client
        .channel("topic")
        .on_broadcast("subtopic", |msg| {
            println!("[BROADCAST RECV] {:?}", msg);
        })
        .build(&mut client);

    let _channel_a = client.block_until_subscribed(channel_a).unwrap();

    let channel_b = client
        .channel("topic")
        .broadcast(BroadcastConfig {
            broadcast_self: true,
            ..Default::default()
        })
        .build(&mut client);
    let channel_b = client.block_until_subscribed(channel_b).unwrap();

    // Send message
    // TODO into into into into, just abstract it away

    let mut payload = HashMap::new();
    payload.insert("message".into(), "hello, broadcast!".into());

    let message = BroadcastPayload::new("subtopic", payload);

    let _ = client
        .get_channel_mut(channel_b)
        .unwrap()
        .broadcast(message);

    loop {
        match client.next_message() {
            Ok(topic) => {
                println!("Message forwarded to {:?}", topic)
            }
            Err(NextMessageError::WouldBlock) => {}
            Err(_e) => {
                //println!("NextMessageError: {:?}", e)
            }
        }
    }
}
