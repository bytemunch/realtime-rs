use std::{collections::HashMap, env};

use realtime_rs::{
    message::{
        message_filter::{MessageFilter, MessageFilterEvent},
        payload::{BroadcastPayload, Payload},
        realtime_message::{MessageEvent, RealtimeMessage},
    },
    sync::{
        realtime_channel::ChannelState,
        realtime_client::{NextMessageError, RealtimeClient},
    },
};

fn main() {
    let url = "ws://127.0.0.1:54321".into();
    let anon_key = env::var("LOCAL_ANON_KEY").expect("No anon key!");

    let mut client = RealtimeClient::new(url, anon_key);

    let _ = client.connect();

    let channel_a = client
        .channel("room-1".into())
        .expect("Channel broke: ")
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

    let channel_b = client
        .channel("room-1".into())
        .expect("Channel broke")
        .subscribe();

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
