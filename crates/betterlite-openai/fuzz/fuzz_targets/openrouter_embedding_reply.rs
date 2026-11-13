#![no_main]

use betterlite_openai::openrouter_embedding::decode_embedding_response;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _reply = decode_embedding_response(bytes);
});
