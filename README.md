# Supabase realtime-rs

Synchronous websocket client wrapper for Supabase realtime. WIP, API is solid as water.

## Progress

### Working so far

[x] Websocket client
    soon to have such use as `RealtimeClient::connect(endpoint)`, currently uses ENV vars, messy and hacky but was quick to implement 
[x] Channels
    `realtime-js` has much more involved channels than i do, i wonder if i'm missing something. i mean more than is obviously missing
[x] Granular Callbacks (`INSERT`, `UPDATE`, `DELETE` and `ALL` (`*`))
    i had to learn what a `Box<dyn T>` is for this one, it was horrible
[x] Heartbeat
    i <3 threads

### TODOs

[ ] Drop callbacks from channels
[ ] Disconnecting Channels
[ ] Disconnecting client
[ ] Configurable heartbeat interval
[ ] Learn what a message ref does and who should set it
[ ] Presence (i don't have the first clue here, research day incoming)
[ ] Broadcast (as above)
[ ] Async client
[ ] Lock down a clean API
[ ] Anything else I can find to do before writing tests
[ ] Tests

# Contributing

Once I've filled the role that other realtime clients do I'll be open to extra contribution, in the mean time it's all duct tape and brute force so suggestions and PRs, while welcomed, may not be satisfactorily implemented.

# LICENSE

MIT / Apache 2, you know the drill it's a Rust project.
