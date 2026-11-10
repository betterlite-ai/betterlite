#![no_main]

use std::str;

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
    let mut remaining = bytes;
    while !remaining.is_empty() {
        match str::from_utf8(remaining) {
            Ok(valid) => {
                text.push_str(valid);
                break;
            }
            Err(error) => {
                let valid_end = error.valid_up_to();
                if let Some(valid) = remaining
                    .get(..valid_end)
                    .and_then(|slice| str::from_utf8(slice).ok())
                {
                    text.push_str(valid);
                }
                text.push(char::REPLACEMENT_CHARACTER);
                let skip = error.error_len().unwrap_or(remaining.len());
                let Some(rest) = remaining.get(valid_end.saturating_add(skip)..) else {
                    break;
                };
                remaining = rest;
            }
        }
    }
    text
}
