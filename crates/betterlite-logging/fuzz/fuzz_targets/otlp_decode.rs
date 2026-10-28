#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|body: &[u8]| {
    let _ = betterlite_logging::otlp_normalize::decode_content_encoding(body, None);
});
