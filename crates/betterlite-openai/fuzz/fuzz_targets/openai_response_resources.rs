#![no_main]

use betterlite_openai::response_resources::response::decode_response_resource;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _decoded = decode_response_resource(bytes);
});
