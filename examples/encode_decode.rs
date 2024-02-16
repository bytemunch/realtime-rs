use std::{collections::HashMap, env};

use realtime_rs::{
    message::payload::{BroadcastConfig, BroadcastPayload, Payload},
    realtime_channel::RealtimeChannelBuilder,
    realtime_client::{ClientState, RealtimeClientBuilder},
};

fn main() {
    let url = "http://127.0.0.1:54321";
    let anon_key = env::var("LOCAL_ANON_KEY").expect("No anon key!");

    fn reverse(s: String) -> String {
        s.chars()
            .rev()
            .collect::<String>()
            .strip_prefix("\"")
            .unwrap()
            .strip_suffix("\"")
            .unwrap()
            .to_string()
    }

    let client = RealtimeClientBuilder::new(url, anon_key)
        .set_encoder(|mut msg| {
            println!("Encoder running...");
            match msg.payload {
                Payload::Broadcast(ref mut payload) => {
                    let mut message: String =
                        payload.payload.get_mut("message").unwrap().to_string();

                    message = reverse(message);

                    payload.payload.insert("message".into(), message.into());

                    return msg;
                }
                _ => {}
            }

            msg
        })
        .set_decoder(|mut msg| {
            println!("Decoder running...");
            match msg.payload {
                Payload::Broadcast(ref mut payload) => {
                    let mut message = payload.payload.get_mut("message").unwrap().to_string();

                    message = reverse(message);

                    payload.payload.insert("message".into(), message.into());

                    return msg;
                }
                _ => {}
            }

            msg
        })
        .connect()
        .to_sync();

    let channel = RealtimeChannelBuilder::new("reverse_encoder")
        .set_broadcast_config(BroadcastConfig {
            broadcast_self: true,
            ack: Default::default(),
        })
        .on_broadcast("test", |b| println!("[BC RECV] {:?}", b.get("message")))
        .build_sync(&client)
        .unwrap();

    channel.subscribe_blocking().unwrap();

    let mut test_payload = HashMap::new();

    test_payload.insert(
        "message".into(),
        serde_json::to_value("Reverse me!").unwrap(),
    );

    let test_payload = BroadcastPayload::new("test", test_payload);

    channel.broadcast(test_payload);

    loop {
        if client.get_state().unwrap() == ClientState::Closed {
            break;
        }
    }

    println!("Client closed.");
}
