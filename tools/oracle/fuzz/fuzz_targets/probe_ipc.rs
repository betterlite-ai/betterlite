#![no_main]
use betterlite_oracle::routing_probe_ipc::{ProbeReply, ProbeRequest, parse_message};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _request = parse_message::<ProbeRequest>(bytes);
    let _reply = parse_message::<ProbeReply>(bytes);
});
