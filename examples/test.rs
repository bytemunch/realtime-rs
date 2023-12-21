use realtime_rs::realtime_client::RealtimeClient;

const LOCAL: bool = true;
const LOCAL_PORT: isize = 54321;

fn main() {
    let client = RealtimeClient::connect(LOCAL, LOCAL_PORT);

    println!("Client created: {:?}", client);
}
