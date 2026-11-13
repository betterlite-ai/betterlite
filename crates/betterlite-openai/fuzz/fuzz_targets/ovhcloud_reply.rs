#![no_main]
use betterlite_openai::openai_base_config::Headers;
use betterlite_openai::ovhcloud_chat::chunk::chunk_parser;
use betterlite_openai::ovhcloud_embedding::reply::decode_embedding_reply;
use betterlite_prompts::arguments::json::Object;
use libfuzzer_sys::fuzz_target;
fuzz_target!(|bytes: &[u8]| {
    let _reply = decode_embedding_reply(bytes, 200, &Headers::new());
    let _chunk = serde_json::from_slice::<serde_json::Value>(bytes)
        .ok()
        .and_then(|value| serde_json::from_value::<Object>(value).ok())
        .map(|chunk| chunk_parser(&chunk));
});
