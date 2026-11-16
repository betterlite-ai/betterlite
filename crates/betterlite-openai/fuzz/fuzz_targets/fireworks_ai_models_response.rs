#![no_main]

use betterlite_openai::fireworks_ai_chat::models::decode_models_response;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _decoded = decode_models_response(200, bytes);
    let _refused = decode_models_response(503, bytes);
});
