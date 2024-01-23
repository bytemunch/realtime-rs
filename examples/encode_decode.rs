use std::{collections::HashMap, env};

use realtime_rs::{
    message::payload::{BroadcastConfig, BroadcastPayload, Payload},
    sync::realtime_client::{NextMessageError, RealtimeClient},
};

fn main() {
    let url = "http://127.0.0.1:54321".into();
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

    let mut client = RealtimeClient::builder(url, anon_key)
        .encode(|mut msg| {
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
        .decode(|mut msg| {
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
        .build();

    match client.connect() {
        Ok(_) => {}
        Err(e) => panic!("Couldn't connect! {:?}", e),
    };

    let channel_id = client
        .channel("reverse_encoder".into())
        .broadcast(BroadcastConfig {
            broadcast_self: true,
            ack: Default::default(),
        })
        .on_broadcast("test".into(), |b| {
            println!("[BC RECV] {:?}", b.get("message"))
        })
        .build(&mut client);

    let _ = client.block_until_subscribed(channel_id);

    let mut test_payload = HashMap::new();

    test_payload.insert(
        "message".into(),
        serde_json::to_value("Reverse me!").unwrap(),
    );

    let _ = client
        .get_channel_mut(channel_id)
        .unwrap()
        .broadcast(BroadcastPayload::new("test".into(), test_payload));

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
