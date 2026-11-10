#![no_main]

use betterlite_logging::passthrough_logging_state::usage::{
    parse_response_cost, parse_total_tokens,
};
use libfuzzer_sys::fuzz_target;

const MAX_BYTES: usize = 64 * 1024;

fuzz_target!(|data: &[u8]| {
    let Some(bytes) = data.get(..data.len().min(MAX_BYTES)) else {
        return;
    };
    let text = lossy_text(bytes);
    let _cost = parse_response_cost(Some(&text));
    let _tokens = parse_total_tokens(Some(&text));
});

fn lossy_text(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len());
    for chunk in bytes.utf8_chunks() {
        text.push_str(chunk.valid());
        if !chunk.invalid().is_empty() {
            text.push(char::REPLACEMENT_CHARACTER);
        }
    }
    text
}
