#![no_main]

use betterlite_openai::moderation_response::ModerationResponse;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    if let Ok(response) = ModerationResponse::parse(bytes) {
        let _json = serde_json::to_vec(&response);
    }
});
