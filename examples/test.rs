// let's start throwing stuff at the wall n see what sticks

use realtime_rs::constants::ChannelEvent;
use serde::{Deserialize, Serialize};
use std::env;
use tungstenite::{client::IntoClientRequest, connect, http::HeaderValue, Message};

const LOCAL: bool = false;
const LOCAL_PORT: isize = 3012;

#[derive(Serialize, Deserialize)]
struct RealtimeMessage {
    event: ChannelEvent,
    topic: String,
    // TODO payload structure
    payload: Payload,
    #[serde(rename = "ref")]
    message_ref: String,
}

#[derive(Serialize, Deserialize)]
enum Payload {
    #[serde(rename = "config")]
    Join(JoinConfig),
}

#[derive(Serialize, Deserialize)]
struct JoinConfig {
    broadcast: JoinConfigBroadcast,
    presence: JoinConfigPresence,
    postgres_changes: Vec<JoinConfigPostgresChangeQuery>,
}

#[derive(Serialize, Deserialize)]
struct JoinConfigBroadcast {
    #[serde(rename = "self")]
    broadcast_self: bool,
    ack: bool,
}

#[derive(Serialize, Deserialize)]
struct JoinConfigPresence {
    key: String,
}

#[derive(Serialize, Deserialize)]
enum PostgresEvent {
    #[serde(rename = "*")]
    All,
    #[serde(rename = "INSERT")]
    Insert,
    #[serde(rename = "UPDATE")]
    Update,
    #[serde(rename = "DELETE")]
    Delete,
}

#[derive(Serialize, Deserialize)]
struct JoinConfigPostgresChangeQuery {
    event: PostgresEvent,
    schema: String,
    table: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    filter: Option<String>, // TODO structured filters
}

fn main() {
    let supabase_id = env::var("SUPABASE_ID").unwrap();
    let anon_key = env::var("ANON_KEY").unwrap();
    let _service_key = env::var("SERVICE_KEY").unwrap();

    let local_anon_key = env::var("LOCAL_ANON_KEY").unwrap();
    let _local_service_key = env::var("LOCAL_SERVICE_KEY").unwrap();

    let url = match LOCAL {
        false => {
            format!(
                "wss://{}.supabase.co/realtime/v1/websocket?apikey={}&vsn=1.0.0",
                supabase_id, anon_key
            )
        }
        true => {
            format!(
                "wss://127.0.0.1:{}/realtime/v1/websocket?apikey={}&vsn=1.0.0",
                LOCAL_PORT, local_anon_key
            )
        }
    };

    let mut req = url.into_client_request().unwrap();
    let headers = req.headers_mut();

    let auth: HeaderValue = format!("Bearer {}", anon_key).parse().unwrap();
    headers.insert("Authorization", auth);

    let xci: HeaderValue = format!("realtime-rs/0.1.0").parse().unwrap();
    headers.insert("X-Client-Info", xci);

    println!("Connecting: {:?}", req);

    let conn = connect(req);

    match conn {
        Ok((mut socket, response)) => {
            println!("Connected to the server");
            println!("Response HTTP code: {}", response.status());
            println!("Response contains the following headers:");
            for (ref header, value) in response.headers() {
                println!(
                    "* {}: {}",
                    header,
                    value.to_str().unwrap_or("oops no string")
                );
            }

            let init = RealtimeMessage {
                event: ChannelEvent::Join,
                topic: format!("realtime:{}", "test"),
                payload: Payload::Join(JoinConfig {
                    presence: JoinConfigPresence {
                        key: "test_key".to_owned(),
                    },
                    broadcast: JoinConfigBroadcast {
                        broadcast_self: true,
                        ack: false,
                    },
                    postgres_changes: vec![JoinConfigPostgresChangeQuery {
                        event: PostgresEvent::All,
                        schema: "public".to_owned(),
                        table: "todos".to_owned(),
                        filter: None,
                    }],
                }),
                message_ref: "init".to_owned(),
            };

            let join_message = serde_json::to_string(&init).expect("Json bad");
            println!("\nJoinMessage:\n{}\n", join_message);

            socket.send(Message::Text(join_message)).unwrap();

            loop {
                let msg = socket.read().expect("Error reading message");
                println!("Received: {}", msg);
            }

            // socket.close(None);
        }
        Err(error) => {
            println!("Oops: {}", error);
        }
    }
}
