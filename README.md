# Supabase realtime-rs

Synchronous websocket client wrapper for Supabase realtime. WIP, API is solid as water.

## Progress

### Working so far

 - [x] Websocket client
 - [x] Channels
 - [x] Granular Callbacks (`INSERT`, `UPDATE`, `DELETE` and `ALL` (`*`))
 - [x] Heartbeat
 - [x] Middleware
   > Saw the js client lib offering overrides for system functions and such, I figure middlewares for received messages can fill this gap
 - [x] Broadcast
   > Very basic implementation, so far untested across different devices
 - [x] Client states
 - [x] Disconnecting client
 - [x] Gracefully disconnecting Channels
   > more work and testing needed here
 - [x] Channel states
 - [x] Single threaded client

### TODOs

 - [ ] Auto reconnecting client
 - [ ] Client `set_auth` + cascade through channels
 - [ ] Middleware example
 - [ ] Configurable heartbeat interval
 - [ ] Real world use case example
   > like getting realtime data and doing something in the main loop in response. perhaps an 'updates since connected' counter
   > will probably need an `mpsc` for moving data out of callback closures
 - [ ] Presence (i don't have the first clue here, research day incoming)
 - [ ] Async client
 - [ ] Lock down a clean API
 - [ ] Docs
 - [ ] Anything else I can find to do before writing tests
 - [ ] Tests

# Contributing

Once I've filled the role that other realtime clients do I'll be open to extra contribution, in the mean time it's all duct tape and brute force so suggestions and PRs, while welcomed, may not be satisfactorily implemented.

# LICENSE

MIT / Apache 2, you know the drill it's a Rust project.
