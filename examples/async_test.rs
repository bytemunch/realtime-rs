use std::time::Duration;

use realtime_rs::sync::RealtimeClient;

#[tokio::main]
async fn main() {
    let endpoint = "http://127.0.0.1:54321";
    let access_token = std::env::var("LOCAL_ANON_KEY").unwrap();

    let mut client = RealtimeClient::builder(endpoint, access_token)
        .heartbeat_interval(Duration::from_secs(1))
        .build();

    let _ = client.connect().await;

    let _ = client.channel("TestTopic").build(&mut client).await;

    let _ = client.handle_incoming().await;

    println!("client closed.");
}
