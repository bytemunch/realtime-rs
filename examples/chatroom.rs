// WARNING!
// This example is a spaghetti mess, I was working quickly to get a PoC done.
// Probably FULL of bad practices and anti-patterns
// But it proves the concept of a broadcast-based chatroom.

use std::{
    collections::HashMap,
    env,
    io::{self, stdout, Write},
    sync::{
        mpsc::{self, Receiver},
        Arc, Mutex,
    },
    thread::{self},
};

use realtime_rs::{
    message::{
        payload::{BroadcastConfig, BroadcastPayload},
        presence::PresenceEvent,
    },
    realtime_channel::RealtimeChannelBuilder,
    realtime_client::{ClientState, RealtimeClientBuilder},
};
use regex::Regex;
use serde::Deserialize;

const DEBUG: bool = false;

#[derive(Deserialize)]
struct ChatMessage {
    author: String,
    message: String,
}

// Chatroom using presence and broadcast
#[tokio::main]
async fn main() {
    env_logger::init();
    let mut email = String::new();
    let mut password = String::new();
    let alias = Arc::new(Mutex::new(String::new()));

    println!("Welcome to SupaChat!\n");

    if DEBUG {
        email = String::from("test@example.com");
        password = String::from("password");
        let mut a_guard = alias.lock().unwrap();
        *a_guard = "test".into();
    } else {
        println!("Enter email: (blank for anon)");
        io::stdin()
            .read_line(&mut email)
            .expect("couldn't parse email");

        email = email.trim().into();

        println!("Password: (blank for anon)");

        // TODO hide password... maybe a thread to spam "\r " to stdout?
        io::stdin()
            .read_line(&mut password)
            .expect("couldn't parse password");

        password = password.trim().into();

        println!("Choose your alias: ");
        let mut buf = String::new();
        io::stdin()
            .read_line(&mut buf)
            .expect("couldn't parse alias");

        let mut a_guard = alias.lock().unwrap();
        *a_guard = buf.trim().into();
    }

    let url = "http://127.0.0.1:54321";
    let anon_key = env::var("LOCAL_ANON_KEY").expect("No anon key!");

    let mut client = RealtimeClientBuilder::new(url, anon_key).connect();

    println!("Connecting...");

    let mut gotrue = go_true::Client::new("http://192.168.64.5:9999".to_string());

    println!("Logging in...");

    if !email.is_empty() && !password.is_empty() {
        match gotrue
            .sign_in(go_true::EmailOrPhone::Email(email), &password)
            .await
        {
            Ok(session) => {
                client.set_access_token(session.access_token).await.unwrap();
            }
            Err(e) => return println!("Login error: {:?}", e),
        }
    }

    let a_guard = alias.lock().unwrap();

    println!("You are now chatting as [{}]\n\nCommands:\n\t/online\t\tShow online users\n\t/alias NAME\tChange your alias\n\t/quit\t\tExit the chatroom", a_guard);

    let on_broadcast_alias = alias.clone();
    let on_join_alias = alias.clone();
    let on_leave_alias = alias.clone();

    print!("\n{}", prompt(a_guard.as_str()));
    stdout().flush().unwrap();

    let channel = RealtimeChannelBuilder::new("chatroom")
        .set_broadcast_config(BroadcastConfig {
            broadcast_self: true,
            ack: false,
        })
        .on_broadcast("supachat", move |message| {
            // TODO impl From<HashMap<String, Value>> for ChatMessage
            let recieved = ChatMessage {
                message: serde_json::from_value(
                    message
                        .get("message")
                        .expect("malformed ChatMessage")
                        .clone(),
                )
                .expect("deser issue"),
                author: serde_json::from_value(
                    message
                        .get("author")
                        .expect("malformed ChatMessage")
                        .clone(),
                )
                .expect("deser issue"),
            };

            print!("\r[{}]: {}", recieved.author, recieved.message);
            let a_guard = on_broadcast_alias.lock().unwrap();
            if recieved.author == *a_guard {
                // TODO not repeat, count buffer
                print!("\r{}\r{}", " ".repeat(50), prompt(a_guard.as_str()));
            } else {
                print!("\n{}", prompt(a_guard.as_str()));
            }
            stdout().flush().unwrap();
        })
        .on_presence(PresenceEvent::Join, move |_id, _state, joins| {
            for (_id, data) in joins.get_phx_map() {
                print!(
                    "\r{} joined the chatroom.",
                    serde_json::from_value::<String>(data.get("alias").unwrap().clone()).unwrap()
                );
                let a_guard = on_join_alias.lock().unwrap();
                print!("\n{}", prompt(a_guard.as_str()));
                stdout().flush().unwrap();
            }
        })
        .on_presence(PresenceEvent::Leave, move |_id, _state, leaves| {
            for (_id, data) in leaves.get_phx_map() {
                print!(
                    "\r{} has gone to touch grass.",
                    serde_json::from_value::<String>(data.get("alias").unwrap().clone()).unwrap()
                );
                let a_guard = on_leave_alias.lock().unwrap();
                print!("\n{}", prompt(a_guard.as_str()));
                stdout().flush().unwrap();
            }
        })
        .build(&mut client)
        .await
        .unwrap();

    channel.subscribe_blocking().await.unwrap();

    let mut state_data = HashMap::new();
    state_data.insert("alias".into(), serde_json::Value::String(a_guard.clone()));

    channel.track(state_data).await.unwrap();

    let stdin_rx = spawn_stdin_channel();

    drop(a_guard);

    loop {
        if client.get_state().await.unwrap() == ClientState::Closed {
            break;
        }

        match stdin_rx.try_recv() {
            Ok(input) => {
                let regex = Regex::new(r"(\/)([\S]*)$").unwrap();

                if let Some(captures) = regex.captures(input.as_str()) {
                    let (_, [_, command]) = captures.extract();

                    match command {
                        "online" => {
                            print!("\rOnline Users: \n");

                            for (_id, data) in channel.get_presence_state().await.get_phx_map() {
                                println!(
                                    "{}",
                                    serde_json::from_value::<String>(
                                        data.get("alias").unwrap().clone()
                                    )
                                    .unwrap()
                                );
                            }

                            let a_guard = alias.lock().unwrap();
                            print!("\r{}\r{}", " ".repeat(50), prompt(a_guard.as_str()));
                            stdout().flush().unwrap();
                        }
                        "quit" => {
                            print!("\rGoodbye! \n");

                            client.disconnect().await.unwrap();

                            stdout().flush().unwrap();
                        }
                        _ => {
                            println!("Couldn't find command {}", command);
                        }
                    }

                    continue;
                };

                let regex = Regex::new(r"(\/)([\S]*)\s([\S]*)$").unwrap();

                if let Some(captures) = regex.captures(input.as_str()) {
                    let (_, [_, command, arg]) = captures.extract();

                    match command {
                        "alias" => {
                            let mut a_guard = alias.lock().unwrap();
                            *a_guard = arg.to_string();

                            let mut state_data = HashMap::new();
                            state_data.insert(
                                "alias".into(),
                                serde_json::to_value(a_guard.clone()).unwrap(),
                            );
                            channel.track(state_data).await.unwrap();

                            println!("\rYou are now chatting as [{}]", a_guard);
                        }
                        _ => {
                            println!("Couldn't find command {}", command);
                        }
                    }

                    continue;
                };

                let mut payload = HashMap::new();
                payload.insert("message".into(), input.trim().into());

                {
                    let a_guard = alias.lock().unwrap();
                    payload.insert("author".into(), a_guard.trim().into());
                }

                let payload = BroadcastPayload::new("supachat", payload);

                let _ = channel.broadcast(payload);
            }
            Err(_e) => {}
        }
    }
}

fn spawn_stdin_channel() -> Receiver<String> {
    let (tx, rx) = mpsc::channel::<String>();
    thread::spawn(move || loop {
        let mut buffer = String::new();

        io::stdin().read_line(&mut buffer).unwrap();
        // Strip ascii control characters
        let buffer = buffer
            .into_bytes()
            .into_iter()
            .filter(|byte| *byte > 30u8 && *byte != 127u8)
            .collect::<Vec<u8>>();
        tx.send(String::from_utf8(buffer).unwrap().trim().to_string())
            .unwrap();

        // TODO not repeat, count buffer
        print!("\r{}", " ".repeat(50));
        stdout().flush().unwrap();
    });
    rx
}

fn prompt(alias: &str) -> String {
    format!("[{}]: ", alias)
}
