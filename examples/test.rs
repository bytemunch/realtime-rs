use realtime_rs::{
    constants::MessageEvent,
    realtime_client::{
        MessageFilter, MessageFilterEvent, NextMessageError, PostgresEvent, RealtimeClient,
    },
};

const LOCAL: bool = true;
const LOCAL_PORT: isize = 54321;

fn main() {
    let mut client = RealtimeClient::connect(LOCAL, Some(LOCAL_PORT));

    client
        .channel("channel_1".to_string())
        .on(
            MessageEvent::PostgresChanges,
            MessageFilter {
                event: MessageFilterEvent::PostgresCDC(PostgresEvent::All),
                schema: "public".into(),
                table: Some("todos".into()),
                ..Default::default()
            },
            |msg| println!("Channel 1, All:\n{:?}", msg),
        )
        .subscribe();

    println!("Client created: {:?}", client);

    loop {
        match client.next_message() {
            Ok(topic) => {
                println!("Message forwarded to {:?}", topic)
            }
            Err(NextMessageError::WouldBlock) => {}
            Err(e) => {
                println!("NextMessageError: {:?}", e)
            }
        }
    }

    //client.listen(); // spin loop to wait for messages...
}
