#![no_main]

use betterlite_models::ollama_model_metadata::reply::decode_ollama_model_reply;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|body: &[u8]| {
    let _reply = decode_ollama_model_reply(body);
});
