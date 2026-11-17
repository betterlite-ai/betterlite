#![no_main]

use betterlite_server::usage_ai_chat_route::body::decode_body;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|body: &[u8]| {
    let _decoded = decode_body(body);
});
