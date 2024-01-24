use realtime_rs::sync::RealtimeClient;

#[tokio::main]
async fn main() {
    let endpoint = "http://127.0.0.1:54321";
    let access_token = std::env::var("LOCAL_ANON_KEY").unwrap();

    let mut client = RealtimeClient::builder(endpoint, access_token).build();

    client.connect().await;
}
