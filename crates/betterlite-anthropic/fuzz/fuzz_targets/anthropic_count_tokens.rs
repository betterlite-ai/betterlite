#![no_main]

use betterlite_anthropic::count_tokens::wire::decode_count_response;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: &[u8]| {
    let _decoded = decode_count_response(input);
});
