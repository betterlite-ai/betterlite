#![no_main]

use betterlite_openai::stream::SseParser;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let mut parser = SseParser::default();
    let _events = parser.push(bytes);
    let _remainder = parser.finish();
});
