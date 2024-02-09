# Supabase realtime-rs

Async + sync websocket client wrappers for Supabase realtime.

## Installation

Crate coming soon, unofficial for now :)

## Usage

### Sync API

`examples/broadcast_sync.rs`
```rs
use std::{collections::HashMap, thread::sleep, time::Duration};

use realtime_rs::{
    message::payload::BroadcastConfig, realtime_channel::RealtimeChannelBuilder,
    realtime_client::RealtimeClientBuilder,
};

fn main() {
    let endpoint = "http://127.0.0.1:54321";
    let access_token = std::env::var("LOCAL_ANON_KEY").unwrap();

    let client = RealtimeClientBuilder::new(endpoint, access_token)
        .heartbeat_interval(Duration::from_secs(29))
        .connect()
        .to_sync();

    let channel = RealtimeChannelBuilder::new("TestTopic")
        .broadcast(BroadcastConfig {
            broadcast_self: true,
            ack: false,
        })
        .on_broadcast("test_event", |map| println!("Event get! {:?}", map))
        .build_sync(&client)
        .unwrap();

    channel.subscribe();

    let mut payload = realtime_rs::message::payload::BroadcastPayload {
        event: "test_event".into(),
        payload: HashMap::new(),
        ..Default::default()
    };

    let mut count = 0;

    loop {
        count += 1;
        println!("SENDING {}", count);
        payload.payload.insert("count".into(), count.into());
        let _ = channel.broadcast(payload.clone());

        sleep(Duration::from_millis(1000));
    }
}
```

### Async API

Internally the crate uses `tokio`.

`examples/broadcast_async.rs`
```rs
use std::{collections::HashMap, time::Duration};

use realtime_rs::{
    message::payload::BroadcastConfig, realtime_channel::RealtimeChannelBuilder,
    realtime_client::RealtimeClientBuilder,
};
use tokio::time::sleep;

#[tokio::main]
async fn main() {
    let endpoint = "http://127.0.0.1:54321";
    let access_token = std::env::var("LOCAL_ANON_KEY").unwrap();

    let client = RealtimeClientBuilder::new(endpoint, access_token)
        .heartbeat_interval(Duration::from_secs(29))
        .connect();

    let channel = RealtimeChannelBuilder::new("TestTopic")
        .broadcast(BroadcastConfig {
            broadcast_self: true,
            ack: false,
        })
        .on_broadcast("test_event", |map| println!("Event get! {:?}", map))
        .build(&client)
        .await
        .unwrap();

    channel.subscribe_blocking().await.unwrap();

    let mut payload = realtime_rs::message::payload::BroadcastPayload {
        event: "test_event".into(),
        payload: HashMap::new(),
        ..Default::default()
    };

    let mut count = 0;

    tokio::spawn(async move {
        loop {
            count += 1;
            println!("SENDING {}", count);
            payload.payload.insert("count".into(), count.into());
            let _ = channel.broadcast(payload.clone());
            sleep(Duration::from_millis(1000)).await;
        }
    })
    .await
    .unwrap();
}
```

See `/examples` for more!

## TODOs

 - [ ] Connection timeouts
 - [ ] Doctestable examples
 - [ ] Custom middlewarey message mutating functions
 - [ ] REST channel sending
 - [ ] Remove unused `derive`s
    > means implementing a bunch of `Serialize` and `Deserialize` traits by hand.. busywork
 - [ ] Throttling

 #### Examples

 - [ ] Example: Act on system messages && test what happens when we ignore them? Need to handle couldn't subscribe errors.

 #### Middleware

 - [ ] Middleware ordering
 - [ ] Middleware example (?) try using current API see if middleware needed
 - [ ] Middleware filtering by `MessageEvent`

# Contributing

Suggestions and PRs are welcomed!

Feel free to open an issue with any bugs, suggestions or ideas.

To make a PR please clone, branch and then pull request against the repo.

# LICENSE

MIT / Apache 2, you know the drill it's a Rust project.
