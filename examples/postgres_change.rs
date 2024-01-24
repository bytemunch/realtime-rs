use std::{cell::RefCell, collections::HashMap, env, rc::Rc};

use realtime_rs::{
    message::{payload::PostgresChangesEvent, PostgresChangeFilter},
    sync::{ConnectionState, NextMessageError, RealtimeClient},
};

#[tokio::main]
async fn main() {
    let url = "http://127.0.0.1:54321";
    let anon_key = env::var("LOCAL_ANON_KEY").expect("No anon key!");

    let event_counter = Rc::new(RefCell::new(0));

    let mut gotrue = go_true::Client::new("http://192.168.64.7:9999".to_string());

    let Ok(session) = gotrue
        .sign_in(
            go_true::EmailOrPhone::Email("test@example.com".into()),
            &String::from("password"),
        )
        .await
    else {
        return println!("Login error");
    };

    let mut client = RealtimeClient::builder(url, anon_key)
        .access_token(session.access_token)
        .build();

    match client.connect() {
        Ok(_) => {}
        Err(e) => panic!("Couldn't connect! {:?}", e), // TODO retry routine
    };

    let rc = Rc::clone(&event_counter);

    let on_change = move |msg: &_| {
        println!("CHANGE");
        rc.replace_with(|&mut count| count + 1);
        println!("Event #{} | Channel 1:\n{:?}", rc.borrow(), msg);
    };

    let channel_id = client
        .channel("topic")
        .on_postgres_change(
            PostgresChangesEvent::All,
            PostgresChangeFilter {
                schema: "public".into(),
                table: Some("todos".into()),
                ..Default::default()
            },
            on_change,
        )
        .presence(realtime_rs::message::payload::PresenceConfig {
            key: Some("test_key".into()),
        })
        .build(&mut client);

    let _ = client.block_until_subscribed(channel_id);

    client
        .get_channel_mut(channel_id)
        .unwrap()
        .track(HashMap::new());

    loop {
        if client.get_status() == ConnectionState::Closed {
            break;
        }

        match client.next_message() {
            Ok(topic) => {
                println!("Message forwarded to {:?}", topic)
            }
            Err(NextMessageError::WouldBlock) => {}
            Err(e) => {
                panic!("NextMessageError: {:?}", e);
            }
        }
    }

    println!("Client closed.");
}
