#![no_main]

use betterlite_openai::vercel_ai_gateway::embedding::decode_embedding_response;
use betterlite_openai::vercel_ai_gateway::models::parse_models;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _reply = decode_embedding_response(bytes);
    let _ok = parse_models(200, bytes);
    let _failed = parse_models(503, bytes);
});
