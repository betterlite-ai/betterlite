#![no_main]

use betterlite_openai::response_events::event_decode::decode_response_event;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _decoded = decode_response_event(bytes);
});
