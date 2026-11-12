#![no_main]
use betterlite_config::tool_prices::{OPENAI_FILE_SEARCH_ENV, decode_tool_price};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(text) = str::from_utf8(data) {
        let _price = decode_tool_price(OPENAI_FILE_SEARCH_ENV, text);
    }
});
