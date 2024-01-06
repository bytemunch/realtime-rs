use std::env;

use realtime_rs::{
    constants::MessageEvent,
    realtime_client::{
        MessageFilter, MessageFilterEvent, NextMessageError, PostgresEvent, RealtimeClient,
    },
};

fn main() {
    let url = "ws://127.0.0.1:54321".into();
    let anon_key = env::var("LOCAL_ANON_KEY").expect("No anon key!");

    let mut client = RealtimeClient::connect(url, anon_key);

    client
        .channel("channel_1".to_string())
        .on(
            MessageEvent::PostgresChanges,
            MessageFilter {
                event: MessageFilterEvent::PostgresCDC(PostgresEvent::All),
                schema: "public".into(),
                table: Some("todos".into()),
                ..Default::default()
            },
            |msg| println!("Channel 1, All:\n{:?}", msg),
        )
        .subscribe();

    println!("Client created: {:?}", client);

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
    }
}
