#![no_main]

use betterlite_foundry::agent_results::event::decode_agent_event;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _decoded = decode_agent_event(bytes);
});
