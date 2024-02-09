use std::{collections::HashMap, env};

use realtime_rs::{
    message::presence::PresenceEvent,
    realtime_channel::RealtimeChannelBuilder,
    realtime_client::{ClientState, RealtimeClientBuilder},
};

fn main() {
    let url = "http://127.0.0.1:54321";
    let anon_key = env::var("LOCAL_ANON_KEY").expect("No anon key!");

    let client = RealtimeClientBuilder::new(url, anon_key)
        .connect()
        .to_sync();

    let mut presence_payload = HashMap::new();
    presence_payload.insert("kb_layout".into(), "en_US".into());

    let channel = RealtimeChannelBuilder::new("channel_1")
        // TODO presence_state message event
        .on_presence(PresenceEvent::Sync, |key, _old_state, _new_state| {
            println!("Presence sync: {:?}", key);
        })
        .on_presence(PresenceEvent::Join, |key, _old_state, _new_state| {
            println!("Presence join: {:?}", key);
        })
        .on_presence(PresenceEvent::Leave, |key, _old_state, _new_state| {
            println!("Presence leave: {:?}", key);
        })
        .build_sync(&client)
        .unwrap();

    channel.subscribe_blocking().unwrap();

    channel.track(presence_payload.clone()).unwrap();

    loop {
        if client.get_state().unwrap() == ClientState::Closed {
            break;
        }
    }

    println!("Client closed.");
}
