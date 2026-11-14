#![no_main]

use betterlite_openai::text_completion_response::CompletionResponse;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _response = serde_json::from_slice::<CompletionResponse>(bytes);
});
