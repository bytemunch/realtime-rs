use std::{
    collections::HashMap,
    env,
    io::{self, stdout, Write},
    sync::mpsc::{self, Receiver},
    thread::{self, sleep},
    time::Duration,
};

use realtime_rs::{
    message::payload::{BroadcastConfig, BroadcastPayload},
    sync::{
        realtime_client::{NextMessageError, RealtimeClient},
        realtime_presence::PresenceEvent,
    },
    DEBUG,
};
use regex::Regex;
use serde::Deserialize;

#[derive(Deserialize)]
struct ChatMessage {
    author: String,
    message: String,
}

const LOCAL: bool = false;
// Chatroom using presence and broadcast

fn main() {
    let mut email = String::new();
    let mut password = String::new();
    let mut alias = String::new();

    println!("Welcome to SupaChat!\n");

    if DEBUG {
        email = String::from("test@example.com");
        password = String::from("password");
        alias = String::from("test");
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
        io::stdin()
            .read_line(&mut alias)
            .expect("couldn't parse alias");

        alias = alias.trim().into();
    }

    let mut url = "http://127.0.0.1:54321".into();
    let mut anon_key = env::var("LOCAL_ANON_KEY").expect("No anon key!");
    let mut auth_url = "http://192.168.64.6:9999".into();

    if !LOCAL {
        url = format!(
            "https://{}.supabase.co",
            env::var("SUPABASE_ID").expect("no supabase id")
        );
        anon_key = env::var("ANON_KEY").expect("No anon key!");
        auth_url = url.clone();
    }

    let mut client = RealtimeClient::builder(url, anon_key)
        .auth_url(auth_url)
        .build();

    println!("Connecting...");

    match client.connect() {
        Ok(_) => {}
        Err(e) => panic!("Couldn't connect! {:?}", e), // TODO retry routine
    };

    println!("Logging in...");

    if !email.is_empty() && !password.is_empty() {
        match client.sign_in_with_email_password(email, password) {
            Ok(()) => {}
            Err(e) => return println!("Login error: {:?}", e),
        }
    }

    println!(
        "You are now chatting as [{}]\n\nCommands:\n\t/online:\t\tShow online users\n",
        alias
    );

    // BORROW CHECKERRRRRRRRRRRRRR
    // TODO superstruct to hold alias and create prompt?
    // orrrr maybe a static function to print prompt, alias as arg

    let prompt = format!("[{}]: ", alias);
    let my_name = alias.clone();
    let my_prompt = prompt.clone();
    let my_my_prompt = prompt.clone();
    let my_my_my_prompt = prompt.clone();

    print!("\n{}", prompt);
    stdout().flush().unwrap();

    let channel_id = client
        .channel("chatroom".into())
        .broadcast(BroadcastConfig {
            broadcast_self: true,
            ack: false,
        })
        .on_broadcast("supachat".into(), move |message| {
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
            if recieved.author == my_name {
                // TODO not repeat, count buffer
                print!("\r{}\r{}", " ".repeat(50), my_prompt);
            } else {
                print!("\n{}", my_prompt);
            }
            stdout().flush().unwrap();
        })
        .on_presence(PresenceEvent::Join, move |_id, _state, joins| {
            for (_id, data) in joins.get_phx_map() {
                print!(
                    "\r{} joined the chatroom.",
                    serde_json::from_value::<String>(data.get("alias").unwrap().clone()).unwrap()
                );
                print!("\n{}", my_my_prompt);
                stdout().flush().unwrap();
            }
        })
        .on_presence(PresenceEvent::Leave, move |_id, _state, leaves| {
            for (_id, data) in leaves.get_phx_map() {
                print!(
                    "\r{} has gone to touch grass.",
                    serde_json::from_value::<String>(data.get("alias").unwrap().clone()).unwrap()
                );
                print!("\n{}", my_my_my_prompt);
                stdout().flush().unwrap();
            }
        })
        .build(&mut client);

    let _ = client.block_until_subscribed(channel_id);

    let mut state_data = HashMap::new();
    state_data.insert("alias".into(), alias.clone().into());
    client.get_channel_mut(channel_id).track(state_data);

    let stdin_rx = spawn_stdin_channel();

    loop {
        sleep(Duration::from_millis(33));

        match client.next_message() {
            Ok(_uuid) => {}
            Err(NextMessageError::ClientClosed) => {
                println!("Client closed");
            }
            Err(NextMessageError::WouldBlock) => {}
            Err(err) => println!("Client error: {}", err),
        }

        match stdin_rx.try_recv() {
            Ok(input) => {
                let regex = Regex::new(r"(\/)([\S]*)$").unwrap();

                if let Some(captures) = regex.captures(input.as_str()) {
                    let (_, [_, command]) = captures.extract();

                    match command {
                        "online" => {
                            print!("\rOnline Users: \n");

                            for (_id, data) in client
                                .get_channel(channel_id)
                                .presence_state()
                                .get_phx_map()
                            {
                                println!(
                                    "{}",
                                    serde_json::from_value::<String>(
                                        data.get("alias").unwrap().clone()
                                    )
                                    .unwrap()
                                );
                            }

                            print!("\r{}\r{}", " ".repeat(50), prompt);
                            stdout().flush().unwrap();
                        }
                        _ => {
                            println!("Couldn't find command {}", command);
                        }
                    }

                    continue;
                };

                let mut payload = HashMap::new();
                payload.insert("message".into(), input.trim().into());
                payload.insert("author".into(), alias.trim().into());

                let payload = BroadcastPayload::new("supachat".into(), payload);

                let _ = client.get_channel_mut(channel_id).broadcast(payload);
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
