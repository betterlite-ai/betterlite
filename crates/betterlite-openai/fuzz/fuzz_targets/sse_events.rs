#![no_main]

use betterlite_openai::stream::SseParser;
use betterlite_openai::stream::chunk::Chunk;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let mut parser = SseParser::default();
    if let Ok(events) = parser.push(bytes) {
        for event in events {
            let _chunk = Chunk::parse(&event, "fuzz", true);
        }
    }
    let _remainder = parser.finish();
});
