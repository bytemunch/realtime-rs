use realtime_rs::realtime_client::RealtimeClient;

const LOCAL: bool = true;
const LOCAL_PORT: isize = 54321;

fn main() {
    let mut client = RealtimeClient::connect(LOCAL, LOCAL_PORT);

    client.on_message(); // TODO this blocks. Shouldn't block.

    println!("Client created: {:?}", client);
}
