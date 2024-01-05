use realtime_rs::realtime_client::{PostgresChange, PostgresEvent, RealtimeClient};

const LOCAL: bool = true;
const LOCAL_PORT: isize = 54321;

fn main() {
    let mut client = RealtimeClient::connect(LOCAL, Some(LOCAL_PORT));

    let channel_1 = client.channel(
        "channel_1".to_string(),
        vec![PostgresChange {
            event: PostgresEvent::All,
            schema: "public".to_owned(),
            table: "todos".to_owned(),
            ..Default::default()
        }],
    );

    let c1_all = channel_1.on(PostgresEvent::All, |msg| {
        println!("Channel 1, All:\n{:?}", msg)
    });

    let _ = channel_1.on(PostgresEvent::Update, |msg| {
        println!("Channel 1, Update:\n{:?}", msg)
    });

    let _ = channel_1.on(PostgresEvent::Delete, |msg| {
        println!("Channel 1, Delete:\n{:?}", msg)
    });

    let c1_insert = channel_1.on(PostgresEvent::Insert, |msg| {
        println!("Channel 1, Insert:\n{:?}", msg)
    });

    let _ = channel_1.drop(c1_all);
    let _ = channel_1.drop(c1_insert);

    let channel_2 = client.channel(
        "channel_2".to_string(),
        vec![PostgresChange {
            event: PostgresEvent::All,
            schema: "public".to_owned(),
            table: "table_2".to_owned(),
            ..Default::default()
        }],
    );

    let _ = channel_2.on(PostgresEvent::Insert, |msg| {
        println!("Channel 2:\n{:?}", msg)
    });

    client.listen(); // spin loop to wait for messages...

    println!("Client created: {:?}", client);
}
