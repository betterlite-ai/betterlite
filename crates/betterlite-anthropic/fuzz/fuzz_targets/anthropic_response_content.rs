#![no_main]

use betterlite_anthropic::response_content::decode_json_mode_arguments;
use libfuzzer_sys::fuzz_target;
use std::str;

fuzz_target!(|data: &[u8]| {
    if let Ok(arguments) = str::from_utf8(data) {
        let _outcome = decode_json_mode_arguments(arguments);
    }
});
