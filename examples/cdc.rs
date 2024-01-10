use std::env;

use realtime_rs::{
    message::{
        message_filter::{MessageFilter, MessageFilterEvent},
        payload::PostgresEvent,
        realtime_message::MessageEvent,
    },
    sync::realtime_client::{ConnectionState, NextMessageError, RealtimeClient},
};

fn main() {
    let url = "ws://127.0.0.1:54321".into();
    let anon_key = env::var("LOCAL_ANON_KEY").expect("No anon key!");

    let mut client = RealtimeClient::new(url, anon_key);

    let client = match client.connect() {
        Ok(client) => client,
        Err(e) => panic!("Couldn't connect! {:?}", e), // TODO retry routine
    };

    client
        .channel("channel_1".to_string())
        .expect("")
        .on(
            MessageEvent::PostgresChanges,
            MessageFilter {
                event: MessageFilterEvent::PostgresCDC(PostgresEvent::Update),
                schema: "public".into(),
                table: Some("todos".into()),
                ..Default::default()
            },
            |msg| println!("Channel 1, Update:\n{:?}", msg),
        )
        .subscribe();

    println!("Client created: {:?}", client);

    loop {
        if client.status == ConnectionState::Closed {
            break;
        }

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

    println!("Client closed.");
}
