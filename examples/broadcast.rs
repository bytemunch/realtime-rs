use std::{collections::HashMap, env};

use realtime_rs::{
    message::realtime_message::RealtimeMessage,
    sync::realtime_client::{NextMessageError, RealtimeClient},
};

fn main() {
    let url = "ws://127.0.0.1:54321".into();
    let anon_key = env::var("LOCAL_ANON_KEY").expect("No anon key!");

    let mut client = RealtimeClient::new(url, anon_key);

    let _ = client.connect();

    let channel_a = client
        .channel("room-1".into())
        .expect("Channel broke: ")
        .on_broadcast("banana".into(), |msg| {
            println!("[BROADCAST RECV] {:?}", msg);
        })
        .subscribe();

    let _channel_a = client.block_until_subscribed(channel_a).unwrap();

    let channel_b = client
        .channel("room-1".into())
        .expect("Channel broke")
        .subscribe();
    //TODO proper builder pattern for channel, imitating the JS api can only go so far, borrow
    //checker is madge so block until subbed has to be on client
    let channel_b = client.block_until_subscribed(channel_b).unwrap();

    // Send message
    // TODO into into into into, just abstract it away

    let mut payload = HashMap::new();
    payload.insert("message".into(), "hello, broadcast!".into());

    let message = RealtimeMessage::broadcast("banana".into(), payload);

    let _ = client.get_channel(channel_b).unwrap().send(message);

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
