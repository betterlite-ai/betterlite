#![no_main]

use betterlite_vertex::gemini_response::decode::decode_generate_content;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    drop(decode_generate_content(bytes));
});
