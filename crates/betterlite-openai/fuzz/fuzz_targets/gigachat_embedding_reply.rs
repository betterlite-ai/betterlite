#![no_main]
use betterlite_openai::gigachat_embedding::transform_response;
use libfuzzer_sys::fuzz_target;
fuzz_target!(|bytes: &[u8]| {
    let _decoded = transform_response(bytes);
});
