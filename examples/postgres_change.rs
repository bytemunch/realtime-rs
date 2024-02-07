use std::{
    env,
    sync::{Arc, Mutex},
};

use realtime_rs::{
    message::{payload::PostgresChangesEvent, PostgresChangeFilter},
    realtime_channel::RealtimeChannelBuilder,
    realtime_client::{ClientState, RealtimeClientBuilder},
};

fn main() {
    let url = "http://127.0.0.1:54321";
    let anon_key = env::var("LOCAL_ANON_KEY").expect("No anon key!");

    let event_counter = Arc::new(Mutex::new(0));

    let mut gotrue = go_true::Client::new("http://192.168.64.5:9999".to_string());

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    let Ok(session) = rt.block_on({
        gotrue.sign_in(
            go_true::EmailOrPhone::Email("test@example.com".into()),
            &String::from("password"),
        )
    }) else {
        return println!("Auth broke");
    };

    let client = RealtimeClientBuilder::new(url, anon_key)
        .access_token(session.access_token)
        .build()
        .to_sync();

    client.connect();

    let rc = Arc::clone(&event_counter);

    let on_change = move |msg: &_| {
        println!("CHANGE");
        let mut c = rc.lock().unwrap();
        *c += 1;
        println!("Event #{} | Channel 1:\n{:?}", *c, msg);
    };

    let channel = RealtimeChannelBuilder::new("topic")
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
        .build_sync(&client);

    channel.unwrap().subscribe_blocking().unwrap();

    loop {
        if client.get_state().unwrap() == ClientState::Closed {
            break;
        }
    }

    println!("Client closed.");
}
