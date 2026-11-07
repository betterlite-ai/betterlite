#![no_main]
use betterlite_oracle::router_probes::{Reply, Request};
use betterlite_oracle::routing_probe_ipc::parse_message;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _request = parse_message::<Request>(bytes);
    let _reply = parse_message::<Reply>(bytes);
});
