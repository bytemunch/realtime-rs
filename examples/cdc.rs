use std::{collections::HashMap, env};

use realtime_rs::{
    message::{cdc_message_filter::CdcMessageFilter, payload::PostgresChangesEvent},
    sync::realtime_client::{ConnectionState, NextMessageError, RealtimeClient},
};

fn main() {
    let url = "ws://127.0.0.1:54321".into();
    let anon_key = env::var("LOCAL_ANON_KEY").expect("No anon key!");

    let auth_url = "http://192.168.64.6:9999".into();

    let mut client = RealtimeClient::builder(url, anon_key)
        .auth_url(auth_url)
        .build();

    match client.connect() {
        Ok(_) => {}
        Err(e) => panic!("Couldn't connect! {:?}", e), // TODO retry routine
    };

    let channel_id = client
        .channel("topic".into())
        .on_cdc(
            PostgresChangesEvent::All,
            CdcMessageFilter {
                schema: "public".into(),
                table: Some("todos".into()),
                ..Default::default()
            },
            |msg| println!("Channel 1:\n{:?}", msg),
        )
        .build(&mut client);

    let _ = client.block_until_subscribed(channel_id);

    client.get_channel_mut(channel_id).track(HashMap::new());

    client.sign_in_with_email_password("test@example.com".into(), "password".into());

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
