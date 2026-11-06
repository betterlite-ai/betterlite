#![no_main]

use betterlite_openai::count_tokens::handler::decode_input_token_count;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _count = decode_input_token_count(bytes);
});
