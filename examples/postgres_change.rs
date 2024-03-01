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
    env_logger::init();
    let url = "http://127.0.0.1:54321/realtime/v1";
    let anon_key = env::var("LOCAL_ANON_KEY").expect("No anon key!");

    let event_counter = Arc::new(Mutex::new(0));

    let mut gotrue = go_true::Api::new("http://172.27.0.4:9999".to_string());

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    let session = match rt.block_on(async {
        let url = gotrue.get_url_for_provider("google");
        println!("Go here to log in:\n{url}");
        gotrue.provider_sign_in().await
    }) {
        Ok(s) => s,
        Err(e) => return println!("AuthError: {}", e),
    };

    println!("Login success!");

    let client = RealtimeClientBuilder::new(url, anon_key)
        .set_access_token(session.access_token)
        .connect()
        .to_sync();

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
        .set_presence_config(realtime_rs::message::payload::PresenceConfig {
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
