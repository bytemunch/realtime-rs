use realtime_rs::realtime_client::{PostgresChange, PostgresEvent, RealtimeClient};

const LOCAL: bool = true;
const LOCAL_PORT: isize = 54321;

fn main() {
    let mut client = RealtimeClient::connect(LOCAL, LOCAL_PORT);

    let channel_1 = client.channel(
        "channel_1".to_string(),
        vec![PostgresChange {
            event: PostgresEvent::All,
            schema: "public".to_owned(),
            table: "todos".to_owned(),
            ..Default::default()
        }],
    );

    let _ = channel_1.on(PostgresEvent::Update, |msg| {
        println!("Channel 1:\n{:?}", msg)
    });

    let channel_2 = client.channel(
        "channel_2".to_string(),
        vec![PostgresChange {
            event: PostgresEvent::All,
            schema: "public".to_owned(),
            table: "todos".to_owned(),
            ..Default::default()
        }],
    );

    let _ = channel_2.on(PostgresEvent::Update, |msg| {
        println!("Channel 2:\n{:?}", msg)
    });

    client.listen(); // spin loop to wait for messages...

    println!("Client created: {:?}", client);
}
