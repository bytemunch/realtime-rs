// this is where all the client structs and methods go :)
// pretty sure writing client first is the way to go
// but client needs channel
// and channel needs client
//
// client holds multiple channels
// client holds socket connection
// client forwards messages to correct channel(s)
// client manages heartbeat
//
// channels hold ref to their topic
// channels hold ref to presence
// channels forward events to callbacks
// channels register callbacks e.g.:
//
// &mut mychannel;
// fn filter_fn() {};
// fn do_something_rad(msg: RealtimePayload) {}
//  mychannel.on(EventType::Any, filter_fn, do_something_rad);
//
// presence is a black box to me right now i'll deal with that laterr
