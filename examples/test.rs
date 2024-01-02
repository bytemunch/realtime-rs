use realtime_rs::realtime_client::{PostgresEvent, RealtimeClient};

const LOCAL: bool = true;
const LOCAL_PORT: isize = 54321;

fn main() {
    let mut client = RealtimeClient::connect(LOCAL, LOCAL_PORT);

    let channel = client.channel("test".to_string());

    let _ = channel.on(PostgresEvent::Update, |msg| {
        println!("Got the message in the callback baybayyyyy!\n{:?}", msg)
    });

    client.on_message(); // spin loop to wait for messages...

    println!("Client created: {:?}", client);
}
