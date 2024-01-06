use std::{collections::HashMap, env};

use realtime_rs::{
    constants::{ChannelState, MessageEvent},
    realtime_client::{
        BroadcastPayload, MessageFilter, MessageFilterEvent, NextMessageError, Payload,
        RealtimeClient, RealtimeMessage,
    },
};

fn main() {
    let url = "ws://127.0.0.1:54321".into();
    let anon_key = env::var("LOCAL_ANON_KEY").expect("No anon key!");

    let mut client = RealtimeClient::connect(url, anon_key);

    let channel_a = client
        .channel("room-1".into())
        .on(
            MessageEvent::Broadcast,
            MessageFilter {
                event: MessageFilterEvent::Custom("banana".into()),
                ..Default::default()
            },
            |msg| {
                println!("[BROADCAST RECV] {:?}", msg);
            },
        )
        .subscribe();

    let channel_b = client.channel("room-1".into()).subscribe();

    let mut sent_once = false;

    loop {
        match client.next_message() {
            Ok(topic) => {
                println!("Message forwarded to {:?}", topic)
            }
            Err(NextMessageError::WouldBlock) => {}
            Err(e) => {
                println!("NextMessageError: {:?}", e)
            }
        }

        if sent_once {
            client.disconnect();
            continue;
        }

        if client.get_channel(channel_a).unwrap().status == ChannelState::Joined
            && client.get_channel(channel_b).unwrap().status == ChannelState::Joined
        {
            println!("Join finished!");
            let mut payload = HashMap::new();

            payload.insert("message".into(), "hello, broadcast!".into());

            let _ = client
                .get_channel(channel_b)
                .unwrap() // TODO inject topic in channel.send
                .send(RealtimeMessage {
                    event: MessageEvent::Broadcast,
                    topic: "realtime:room-1".into(),
                    payload: Payload::Broadcast(BroadcastPayload {
                        event: "banana".into(),
                        payload,
                        ..Default::default()
                    }),
                    ..Default::default()
                });
            sent_once = true;
        }
    }
}
