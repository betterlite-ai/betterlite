#![no_main]

use betterlite_openai::vercel_ai_gateway::embedding::transform_embedding_response;
use betterlite_openai::vercel_ai_gateway::models::parse_models;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let mut hook = |_text: &str| {};
    let _reply = transform_embedding_response(bytes, &mut hook);
    let _ok = parse_models(200, bytes);
    let _failed = parse_models(503, bytes);
});
