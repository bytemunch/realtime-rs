use std::{collections::HashMap, env};

use realtime_rs::{
    message::payload::BroadcastPayload,
    sync::realtime_client::{NextMessageError, RealtimeClient},
};

fn main() {
    let url = "http://127.0.0.1:54321".into();
    let anon_key = env::var("LOCAL_ANON_KEY").expect("No anon key!");
    let auth_url = "http://192.168.64.6:9999".into();

    let mut client = RealtimeClient::builder(url, anon_key)
        .auth_url(auth_url)
        .build();

    let _ = client.connect();

    let channel_a = client
        .channel("topic".into())
        .on_broadcast("subtopic".into(), |msg| {
            println!("[BROADCAST RECV] {:?}", msg);
        })
        .build(&mut client);

    let _channel_a = client.block_until_subscribed(channel_a).unwrap();

    let channel_b = client.channel("topic".into()).build(&mut client);
    let channel_b = client.block_until_subscribed(channel_b).unwrap();

    // Send message
    // TODO into into into into, just abstract it away

    let mut payload = HashMap::new();
    payload.insert("message".into(), "hello, broadcast!".into());

    let message = BroadcastPayload::new("subtopic".into(), payload);

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
