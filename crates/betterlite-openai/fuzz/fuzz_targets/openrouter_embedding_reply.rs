#![no_main]

use betterlite_openai::openrouter_embedding::transform_embedding_response;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _reply = transform_embedding_response(bytes, &mut |_text| {});
});
