use std::{cell::RefCell, collections::HashMap, env, rc::Rc};

use realtime_rs::{
    message::{payload::PostgresChangesEvent, PostgresChangeFilter},
    sync::{ConnectionState, NextMessageError, RealtimeClient},
};

fn main() {
    let url = "http://127.0.0.1:54321";
    let anon_key = env::var("LOCAL_ANON_KEY").expect("No anon key!");

    let event_counter = Rc::new(RefCell::new(0));

    let mut client = RealtimeClient::builder(url, anon_key).build();

    match client.connect() {
        Ok(_) => {}
        Err(e) => panic!("Couldn't connect! {:?}", e), // TODO retry routine
    };

    match client.sign_in_with_email_password("test@example.com", "password") {
        Ok(()) => {}
        Err(e) => println!("Signin failed: {:?}", e),
    }

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
