#![no_main]

use betterlite_openai::mistral_chat::reply::decode_mistral_chat;
use betterlite_openai::mistral_chat::stream::chunk_parser;
use libfuzzer_sys::fuzz_target;
use serde_json::Value;
use uuid::Uuid;

fuzz_target!(|data: &[u8]| {
    let _reply = decode_mistral_chat(data);
    if let Ok(mut chunk) = serde_json::from_slice::<Value>(data) {
        let _stream = chunk_parser(&mut chunk, Uuid::nil, i64::default);
    }
});
