#![no_main]

use betterlite_openai::volcengine_responses::stream::decode_stream_event;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _decoded = decode_stream_event(bytes);
});
