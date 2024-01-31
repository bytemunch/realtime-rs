# Supabase realtime-rs

Synchronous websocket client wrapper for Supabase realtime. WIP, API is solid as water.

## Progress

### Working so far

#### Core

 - [x] Websocket client
 - [x] Channels
 - [x] Granular Callbacks (`INSERT`, `UPDATE`, `DELETE` and `ALL` (`*`))
 - [x] Heartbeat
 - [x] Client states
 - [x] Disconnecting client
 - [x] Single threaded client
 - [x] Connection timeout + retry
 - [x] Configurable reconnect max attempts
 - [x] Auto reconnect
 - [x] Configurable client-side message throttling
 - [x] TLS websockets
 - [x] Docs
    > - [x] Make `pub(crate)` anything that doesn't need to be user facing
 - [x] Encode / Decode
    > Basic user provided function support.

#### Channels

 - [x] Broadcast
 - [x] Gracefully disconnecting Channels
   > more work and testing needed here
 - [x] Channel states
 - [x] Client `set_auth` + cascade through channels
 - [x] Presence
 - [x] Blocking `.subscribe()`
   > currently implemented on the client due to my skill issues with the borrow checker. plan to move it to channel once im good at coding

#### Extra

 - [x] Middleware
   > Saw the js client lib offering overrides for system functions and such, I figure middlewares for received messages can fill this gap
   > May be useless
 - [x] Placeholder function for email/password auth (immediately deprecated in favour of gotrue-rs or auth-rs or whatever it will be called.)
 - [x] Builder patterns for client and channel
 - [x] Example: CLI broadcast chatroom with active user list (accessed with a `/online` command)
 - [x] Example: Pull data out of closure and modify state in a higher scope
   > `event_counter` in `examples/cdc.rs`, `alias` in `examples/chatroom.rs`. Uses `Rc<RefCell<_>>`.

### TODOs


 - [ ] Async by default, maybe provide sync helper functions
    > The rest of the supabase rs ecosystem is async, so this should be too.
    >> TOKIOOOOOOOOOOOOO

 - [ ] Lock down a clean API
 - [ ] Anything else I can find to do before writing tests
 - [ ] Tests
    > Many cases should be handled with docs code examples
 - [ ] Set up CI
 - [ ] REST channel sending
 - [ ] Remove unused `derive`s
    > means implementing a bunch of `Serialize` and `Deserialize` traits by hand.. busywork

 #### Broken by Async:
  - [ ] Autoreconnect
  - [ ] Block until subscribed
  - [ ] remove all channels
  - [ ] Socket R/W
  - [ ] Disconnect
  - [ ] Throttling
  - [ ] Monitoring

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
