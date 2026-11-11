#![no_main]

use betterlite_core::api::genai_chat_input::{GenerateContentRequest, translate_generate_content};
use betterlite_core::schema::SchemaLimits;
use libfuzzer_sys::fuzz_target;

const MAX_INPUT: usize = 0x0004_0000;

fuzz_target!(|bytes: &[u8]| {
    if bytes.len() > MAX_INPUT {
        return;
    }
    let Ok(request) = serde_json::from_slice::<GenerateContentRequest>(bytes) else {
        return;
    };
    drop(translate_generate_content(
        &request,
        None,
        SchemaLimits::default(),
    ));
});
