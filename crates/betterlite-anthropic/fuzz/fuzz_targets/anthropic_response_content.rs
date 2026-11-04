#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(arguments) = std::str::from_utf8(data) {
        let _ = betterlite_anthropic::response_content::decode_json_mode_arguments(arguments);
    }
});
