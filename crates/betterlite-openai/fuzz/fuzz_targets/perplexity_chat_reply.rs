#![no_main]

use betterlite_openai::perplexity_chat::with_perplexity_fields;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let reply = serde_json::from_slice(bytes).unwrap_or_default();
    let _enhanced = with_perplexity_fields(reply, bytes);
});
